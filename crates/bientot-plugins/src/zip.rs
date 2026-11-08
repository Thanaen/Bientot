//! The zip subset of `.bientot-plugin` files (contract `package.md`, « Reader rules »). The reader
//! never extracts to disk: it reads the central directory, checks every entry against its local
//! header, and inflates into memory under size caps. The writer makes the same subset.

use std::io::{Read, Write};

use flate2::Crc;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;
const FLAG_ENCRYPTED: u16 = 1;
const FLAG_DESCRIPTOR: u16 = 1 << 3;
const FLAG_STRONG_ENCRYPTION: u16 = 1 << 6;
const FLAG_UTF8: u16 = 1 << 11;

pub const MAX_PACKAGE_BYTES: usize = 8 << 20;
pub const MAX_ENTRIES: usize = 256;
pub const MAX_RATIO: u64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipError {
    TooLarge,
    NotZip,
    Zip64,
    TooManyEntries,
    Truncated,
    HeaderMismatch(String),
    Encrypted(String),
    Method(String),
    Symlink(String),
    NotAscii,
    Overlap,
    EntryTooLarge(String),
    Ratio(String),
    Corrupt(String),
}

/// One entry, as the central directory names it, before inflation.
struct Central {
    name: String,
    method: u16,
    flags: u16,
    crc: u32,
    compressed: u64,
    size: u64,
    offset: u64,
    symlink: bool,
}

/// Reads every entry into memory. `limit(name)` gives the largest uncompressed size an entry may
/// have, or `None` for a name the package does not accept, which the caller reports.
pub fn read(
    bytes: &[u8],
    limit: impl Fn(&str) -> Option<u64>,
) -> Result<Vec<(String, Vec<u8>)>, ZipError> {
    if bytes.len() > MAX_PACKAGE_BYTES {
        return Err(ZipError::TooLarge);
    }
    let end = find_end(bytes)?;
    let at = |offset: usize, len: usize| bytes.get(offset..offset.checked_add(len)?);
    let record = at(end, 22).ok_or(ZipError::Truncated)?;
    let (disk, cd_disk) = (u16_at(record, 4), u16_at(record, 6));
    let (on_disk, total) = (u16_at(record, 8), u16_at(record, 10));
    let (cd_size, cd_offset) = (u32_at(record, 12), u32_at(record, 16));
    if end >= 20 && u32_at(&bytes[end - 20..], 0) == ZIP64_LOCATOR {
        return Err(ZipError::Zip64);
    }
    if total == 0xffff || cd_size == u32::MAX || cd_offset == u32::MAX {
        return Err(ZipError::Zip64);
    }
    if disk != 0 || cd_disk != 0 || on_disk != total {
        return Err(ZipError::NotZip);
    }
    if usize::from(total) > MAX_ENTRIES {
        return Err(ZipError::TooManyEntries);
    }
    let (cd_offset, cd_size) = (cd_offset as usize, cd_size as usize);
    if cd_offset.checked_add(cd_size) != Some(end) {
        return Err(ZipError::Corrupt("central directory".into()));
    }

    let mut entries = Vec::with_capacity(usize::from(total));
    let mut position = cd_offset;
    for _ in 0..total {
        let header = at(position, 46).ok_or(ZipError::Truncated)?;
        if u32_at(header, 0) != CENTRAL {
            return Err(ZipError::Corrupt("central directory".into()));
        }
        let made_by = u16_at(header, 4);
        let name_len = usize::from(u16_at(header, 28));
        let extra_len = usize::from(u16_at(header, 30));
        let comment_len = usize::from(u16_at(header, 32));
        let raw_name = at(position + 46, name_len).ok_or(ZipError::Truncated)?;
        if !raw_name.is_ascii() {
            return Err(ZipError::NotAscii);
        }
        let name = String::from_utf8_lossy(raw_name).into_owned();
        let external = u32_at(header, 38);
        // Unix-made entries carry the file mode in the high half of the external attributes.
        let symlink = made_by >> 8 == 3 && (external >> 16) & 0o170000 == 0o120000;
        let compressed = u32_at(header, 20);
        let size = u32_at(header, 24);
        let offset = u32_at(header, 42);
        if compressed == u32::MAX || size == u32::MAX || offset == u32::MAX {
            return Err(ZipError::Zip64);
        }
        entries.push(Central {
            name,
            method: u16_at(header, 10),
            flags: u16_at(header, 8),
            crc: u32_at(header, 16),
            compressed: u64::from(compressed),
            size: u64::from(size),
            offset: u64::from(offset),
            symlink,
        });
        position += 46 + name_len + extra_len + comment_len;
    }
    if position != end {
        return Err(ZipError::Corrupt("central directory".into()));
    }

    let mut spans = Vec::with_capacity(entries.len());
    let mut out = Vec::with_capacity(entries.len());
    for entry in &entries {
        if entry.flags & (FLAG_ENCRYPTED | FLAG_STRONG_ENCRYPTION) != 0 {
            return Err(ZipError::Encrypted(entry.name.clone()));
        }
        if entry.symlink {
            return Err(ZipError::Symlink(entry.name.clone()));
        }
        if entry.method != STORED && entry.method != DEFLATED {
            return Err(ZipError::Method(entry.name.clone()));
        }
        let Some(max) = limit(&entry.name) else {
            out.push((entry.name.clone(), Vec::new()));
            continue;
        };
        if entry.size > max {
            return Err(ZipError::EntryTooLarge(entry.name.clone()));
        }
        if entry.size > 0 && (entry.compressed == 0 || entry.size / entry.compressed > MAX_RATIO) {
            return Err(ZipError::Ratio(entry.name.clone()));
        }

        let offset = entry.offset as usize;
        let local = at(offset, 30).ok_or(ZipError::Truncated)?;
        let mismatch = || ZipError::HeaderMismatch(entry.name.clone());
        if u32_at(local, 0) != LOCAL {
            return Err(mismatch());
        }
        let local_flags = u16_at(local, 6);
        let name_len = usize::from(u16_at(local, 26));
        let extra_len = usize::from(u16_at(local, 28));
        let local_name = at(offset + 30, name_len).ok_or(ZipError::Truncated)?;
        if local_name != entry.name.as_bytes()
            || u16_at(local, 8) != entry.method
            || (local_flags & !FLAG_UTF8) != (entry.flags & !FLAG_UTF8)
        {
            return Err(mismatch());
        }
        // With a data descriptor the local header may leave the CRC and the sizes at zero.
        let (crc, compressed, size) = (u32_at(local, 14), u32_at(local, 18), u32_at(local, 22));
        let described =
            entry.flags & FLAG_DESCRIPTOR != 0 && crc == 0 && compressed == 0 && size == 0;
        if !described
            && (crc != entry.crc
                || u64::from(compressed) != entry.compressed
                || u64::from(size) != entry.size)
        {
            return Err(mismatch());
        }
        let data_start = offset + 30 + name_len + extra_len;
        let data = at(data_start, entry.compressed as usize).ok_or(ZipError::Truncated)?;
        spans.push((offset, data_start + data.len()));

        let content = match entry.method {
            STORED => data.to_vec(),
            _ => {
                let mut content = Vec::with_capacity(entry.size as usize);
                DeflateDecoder::new(data)
                    .take(entry.size + 1)
                    .read_to_end(&mut content)
                    .map_err(|_| ZipError::Corrupt(entry.name.clone()))?;
                content
            }
        };
        if content.len() as u64 != entry.size {
            return Err(ZipError::Corrupt(entry.name.clone()));
        }
        let mut check = Crc::new();
        check.update(&content);
        if check.sum() != entry.crc {
            return Err(ZipError::Corrupt(entry.name.clone()));
        }
        out.push((entry.name.clone(), content));
    }

    spans.sort_unstable();
    if spans.windows(2).any(|pair| pair[0].1 > pair[1].0)
        || spans.last().is_some_and(|last| last.1 > cd_offset)
    {
        return Err(ZipError::Overlap);
    }
    Ok(out)
}

fn find_end(bytes: &[u8]) -> Result<usize, ZipError> {
    if bytes.len() < 22 {
        return Err(ZipError::NotZip);
    }
    // The record is 22 bytes plus a comment of at most 65 535 bytes.
    let lowest = bytes.len().saturating_sub(22 + 0xffff);
    (lowest..=bytes.len() - 22)
        .rev()
        .find(|&i| {
            u32_at(&bytes[i..], 0) == END
                && i + 22 + usize::from(u16_at(&bytes[i..], 20)) == bytes.len()
        })
        .ok_or(ZipError::NotZip)
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Writes `entries` deflated (stored when empty), in the given order, with a fixed timestamp so
/// that the same files give the same package.
pub fn write(entries: &[(&str, &[u8])]) -> Vec<u8> {
    const VERSION: u16 = 20;
    const TIME: u16 = 0;
    const DATE: u16 = (2026 - 1980) << 9 | 1 << 5 | 1;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for &(name, data) in entries {
        let mut crc = Crc::new();
        crc.update(data);
        let (method, body) = if data.is_empty() {
            (STORED, Vec::new())
        } else {
            let mut encoder = DeflateEncoder::new(Vec::new(), flate2::Compression::best());
            encoder.write_all(data).expect("writing to memory");
            (DEFLATED, encoder.finish().expect("writing to memory"))
        };
        let offset = out.len() as u32;
        let common = |buffer: &mut Vec<u8>| {
            put16(buffer, 0); // flags
            put16(buffer, method);
            put16(buffer, TIME);
            put16(buffer, DATE);
            put32(buffer, crc.sum());
            put32(buffer, body.len() as u32);
            put32(buffer, data.len() as u32);
            put16(buffer, name.len() as u16);
            put16(buffer, 0); // extra
        };
        put32(&mut out, LOCAL);
        put16(&mut out, VERSION);
        common(&mut out);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&body);

        put32(&mut central, CENTRAL);
        put16(&mut central, VERSION);
        put16(&mut central, VERSION);
        common(&mut central);
        put16(&mut central, 0); // comment
        put16(&mut central, 0); // disk
        put16(&mut central, 0); // internal attributes
        put32(&mut central, 0); // external attributes
        put32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    put32(&mut out, END);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, entries.len() as u16);
    put16(&mut out, entries.len() as u16);
    put32(&mut out, central.len() as u32);
    put32(&mut out, cd_offset);
    put16(&mut out, 0);
    out
}

fn put16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn put32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn any(_: &str) -> Option<u64> {
        Some(1 << 20)
    }

    #[test]
    fn what_it_writes_it_reads() {
        let zip = write(&[("a.txt", b"hello hello hello"), ("empty", b"")]);
        let entries = read(&zip, any).unwrap();
        assert_eq!(
            entries[0],
            ("a.txt".to_owned(), b"hello hello hello".to_vec())
        );
        assert_eq!(entries[1], ("empty".to_owned(), Vec::new()));
        assert_eq!(write(&[("a", b"x")]), write(&[("a", b"x")]));
    }

    #[test]
    fn caps_apply() {
        let big = vec![b'a'; 2048];
        let zip = write(&[("a", &big)]);
        assert_eq!(
            read(&zip, |_| Some(1024)),
            Err(ZipError::EntryTooLarge("a".into()))
        );
        // 2 KiB of one byte deflates far past a ratio of 100? No: it stays near 100, so use more.
        let bomb = vec![0u8; 1 << 20];
        let zip = write(&[("a", &bomb)]);
        assert_eq!(read(&zip, any), Err(ZipError::Ratio("a".into())));
        assert_eq!(
            read(&vec![0; MAX_PACKAGE_BYTES + 1], any),
            Err(ZipError::TooLarge)
        );
        let names: Vec<String> = (0..257).map(|i| format!("f{i}")).collect();
        let many: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
        assert_eq!(read(&write(&many), any), Err(ZipError::TooManyEntries));
    }

    #[test]
    fn local_headers_must_agree() {
        let mut zip = write(&[("a.txt", b"hello")]);
        // The local name differs from the central one.
        zip[30] = b'b';
        assert_eq!(
            read(&zip, any),
            Err(ZipError::HeaderMismatch("a.txt".into()))
        );
        let mut zip = write(&[("a.txt", b"hello")]);
        // The local CRC differs.
        zip[14] ^= 1;
        assert_eq!(
            read(&zip, any),
            Err(ZipError::HeaderMismatch("a.txt".into()))
        );
    }

    #[test]
    fn corrupt_data_is_caught() {
        let mut zip = write(&[("a.txt", b"hello hello hello hello")]);
        let data_start = 30 + 5;
        zip[data_start + 2] ^= 0x55;
        assert!(matches!(
            read(&zip, any),
            Err(ZipError::Corrupt(_)) | Err(ZipError::Ratio(_))
        ));
    }

    #[test]
    fn flags_and_methods_are_checked() {
        let central_flags = |zip: &mut Vec<u8>, flags: u16| {
            let cd = u32_at(zip, zip.len() - 6) as usize;
            zip[cd + 8..cd + 10].copy_from_slice(&flags.to_le_bytes());
            zip[6..8].copy_from_slice(&flags.to_le_bytes());
        };
        let mut zip = write(&[("a", b"x")]);
        central_flags(&mut zip, FLAG_ENCRYPTED);
        assert_eq!(read(&zip, any), Err(ZipError::Encrypted("a".into())));

        let mut zip = write(&[("a", b"x")]);
        let cd = u32_at(&zip, zip.len() - 6) as usize;
        zip[cd + 10] = 12; // bzip2
        zip[8] = 12;
        assert_eq!(read(&zip, any), Err(ZipError::Method("a".into())));

        let mut zip = write(&[("link", b"target")]);
        let cd = u32_at(&zip, zip.len() - 6) as usize;
        zip[cd + 5] = 3; // made on Unix
        zip[cd + 38..cd + 42].copy_from_slice(&(0o120777u32 << 16).to_le_bytes());
        assert_eq!(read(&zip, any), Err(ZipError::Symlink("link".into())));
    }

    #[test]
    fn non_zip_and_zip64_are_refused() {
        assert_eq!(read(b"not a zip at all, sorry", any), Err(ZipError::NotZip));
        let mut zip = write(&[("a", b"x")]);
        let end = zip.len() - 22;
        zip[end + 10..end + 12].copy_from_slice(&0xffffu16.to_le_bytes());
        zip[end + 8..end + 10].copy_from_slice(&0xffffu16.to_le_bytes());
        assert_eq!(read(&zip, any), Err(ZipError::Zip64));
    }

    #[test]
    fn unknown_names_are_reported_without_inflating() {
        let zip = write(&[("ok", b"x"), ("nope", b"y")]);
        let entries = read(&zip, |name| (name == "ok").then_some(10)).unwrap();
        assert_eq!(entries[1], ("nope".to_owned(), Vec::new()));
    }

    #[test]
    fn non_ascii_names_are_refused() {
        let zip = write(&[("é", b"x")]);
        assert_eq!(read(&zip, any), Err(ZipError::NotAscii));
    }
}
