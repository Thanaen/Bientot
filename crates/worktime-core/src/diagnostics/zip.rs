//! A ZIP archive of a few small entries, as `System.IO.Compression.ZipArchive` writes one on
//! Windows: deflated entries, an empty one stored, sizes in the local headers, no ZIP64, no
//! extra fields and no file attributes. The deflate streams differ from zlib's, the content
//! they hold does not.

use std::io::{self, Write};

use flate2::Compression;
use flate2::Crc;
use flate2::write::DeflateEncoder;
use jiff::civil::DateTime;

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_CENTRAL_DIRECTORY: u32 = 0x0605_4b50;
/// Version 2.0: deflate. The high byte 0 says the archive was made on MS-DOS or Windows.
const VERSION: u16 = 20;
/// The general purpose flags .NET writes for `CompressionLevel.SmallestSize`, on every entry.
/// Names are ASCII and sizes sit in the local headers, so no other flag is set.
const MAXIMUM_COMPRESSION: u16 = 0b10;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;

/// Writes `entries` to `output` with `modified` as every entry's local modification time.
pub(crate) fn write(
    output: &mut impl Write,
    entries: &[(&str, &[u8])],
    modified: DateTime,
) -> io::Result<()> {
    let (time, date) = dos_time(modified);
    let mut offset = 0_u32;
    let mut central = Vec::new();
    for &(name, data) in entries {
        let mut crc = Crc::new();
        crc.update(data);
        // An entry nothing was written to holds no deflate stream, so it is stored.
        let (method, compressed) = if data.is_empty() {
            (STORED, Vec::new())
        } else {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
            encoder.write_all(data)?;
            (DEFLATED, encoder.finish()?)
        };
        let header = Header {
            method,
            time,
            date,
            crc: crc.sum(),
            compressed_size: size(compressed.len())?,
            size: size(data.len())?,
            name,
        };

        let mut local = Vec::with_capacity(30 + name.len());
        put32(&mut local, LOCAL_HEADER);
        put16(&mut local, VERSION);
        header.put(&mut local);
        local.extend_from_slice(name.as_bytes());
        output.write_all(&local)?;
        output.write_all(&compressed)?;

        put32(&mut central, CENTRAL_HEADER);
        put16(&mut central, VERSION);
        put16(&mut central, VERSION);
        header.put(&mut central);
        put16(&mut central, 0); // comment length
        put16(&mut central, 0); // disk number
        put16(&mut central, 0); // internal attributes
        put32(&mut central, 0); // external attributes
        put32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());

        offset = offset
            .checked_add(size(local.len() + compressed.len())?)
            .ok_or_else(too_large)?;
    }
    output.write_all(&central)?;

    let count = u16::try_from(entries.len()).map_err(|_| too_large())?;
    let mut end = Vec::with_capacity(22);
    put32(&mut end, END_OF_CENTRAL_DIRECTORY);
    put16(&mut end, 0); // this disk
    put16(&mut end, 0); // disk of the central directory
    put16(&mut end, count);
    put16(&mut end, count);
    put32(&mut end, size(central.len())?);
    put32(&mut end, offset);
    put16(&mut end, 0); // comment length
    output.write_all(&end)
}

/// The fields the local and central headers share, from the general purpose flags on.
struct Header<'a> {
    method: u16,
    time: u16,
    date: u16,
    crc: u32,
    compressed_size: u32,
    size: u32,
    name: &'a str,
}

impl Header<'_> {
    fn put(&self, out: &mut Vec<u8>) {
        put16(out, MAXIMUM_COMPRESSION);
        put16(out, self.method);
        put16(out, self.time);
        put16(out, self.date);
        put32(out, self.crc);
        put32(out, self.compressed_size);
        put32(out, self.size);
        put16(
            out,
            u16::try_from(self.name.len()).expect("entry names are short"),
        );
        put16(out, 0); // extra field length
    }
}

/// The MS-DOS date and time fields; MS-DOS dates start in 1980, so earlier times are written as
/// its first instant, as .NET does.
fn dos_time(modified: DateTime) -> (u16, u16) {
    let year = i32::from(modified.year());
    if !(1980..=2107).contains(&year) {
        return (0, (1 << 5) | 1);
    }
    let field = |value: i8| u16::try_from(value).expect("civil fields are non-negative");
    let time = (field(modified.hour()) << 11)
        | (field(modified.minute()) << 5)
        | (field(modified.second()) / 2);
    let date = (u16::try_from(year - 1980).expect("checked above") << 9)
        | (field(modified.month()) << 5)
        | field(modified.day());
    (time, date)
}

fn size(length: usize) -> io::Result<u32> {
    u32::try_from(length).map_err(|_| too_large())
}

fn too_large() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "archive needs ZIP64")
}

fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
