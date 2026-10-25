//! Secrets encrypted with DPAPI for the current Windows user, in the files the .NET
//! `DpapiSecretStore` reads and writes: `<directory>/<SHA-256 hex>.secret`.

#![allow(unsafe_code)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::ptr;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

use super::{SecretError, SecretId, SecretStore};
use crate::file;

/// The `secrets` folder of a data directory, as the .NET app uses it.
#[derive(Clone, Debug)]
pub struct DpapiSecretStore {
    directory: PathBuf,
}

impl DpapiSecretStore {
    /// The store for `directory`, made absolute like .NET's `Path.GetFullPath`. Nothing is
    /// created until the first save.
    pub fn new(directory: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            directory: std::path::absolute(directory)?,
        })
    }

    fn path(&self, id: &SecretId) -> PathBuf {
        self.directory.join(id.file_name())
    }
}

impl SecretStore for DpapiSecretStore {
    fn get(&self, id: &SecretId) -> Result<Option<String>, SecretError> {
        let mut protected = match fs::read(self.path(id)) {
            Ok(protected) => protected,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(SecretError::Io(error)),
        };
        let clear = unprotect(&protected, &id.entropy());
        wipe(&mut protected);
        let clear = clear.map_err(SecretError::Unprotect)?;
        Ok(Some(into_string(clear)))
    }

    fn save(&self, id: &SecretId, secret: &str) -> Result<(), SecretError> {
        fs::create_dir_all(&self.directory).map_err(SecretError::Io)?;
        let mut protected =
            protect(secret.as_bytes(), &id.entropy()).map_err(SecretError::Protect)?;
        let replaced = file::replace(&self.path(id), &protected);
        wipe(&mut protected);
        replaced.map_err(SecretError::Io)
    }

    fn delete(&self, id: &SecretId) -> Result<(), SecretError> {
        match fs::remove_file(self.path(id)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(SecretError::Io(error)),
            _ => Ok(()),
        }
    }
}

/// The decrypted bytes as text. .NET writes the UTF-8 of a string, so they always are UTF-8;
/// anything else decodes with replacement characters, as `Encoding.UTF8.GetString` does.
fn into_string(clear: Vec<u8>) -> String {
    String::from_utf8(clear).unwrap_or_else(|error| {
        let text = String::from_utf8_lossy(error.as_bytes()).into_owned();
        wipe(&mut error.into_bytes());
        text
    })
}

/// Overwrites key material before it is dropped, as .NET's `CryptographicOperations.ZeroMemory`.
fn wipe(bytes: &mut [u8]) {
    bytes.fill(0);
    std::hint::black_box(bytes);
}

/// `ProtectedData.Protect(clear, entropy, DataProtectionScope.CurrentUser)`.
fn protect(clear: &[u8], entropy: &[u8; 32]) -> io::Result<Vec<u8>> {
    let input = input_blob(clear)?;
    let entropy = input_blob(entropy)?;
    let mut output = OutputBlob::empty();
    // SAFETY: `input` and `entropy` point to `cbData` readable bytes borrowed for the whole call,
    // and DPAPI only reads them. The description, reserved and prompt pointers may be null.
    // `output.0` is a valid blob DPAPI fills with a `LocalAlloc` buffer, which `OutputBlob` frees.
    let succeeded = unsafe {
        CryptProtectData(
            &input,
            ptr::null(),
            &entropy,
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output.0,
        )
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(output.bytes().to_vec())
}

/// `ProtectedData.Unprotect(protected, entropy, DataProtectionScope.CurrentUser)`.
fn unprotect(protected: &[u8], entropy: &[u8; 32]) -> io::Result<Vec<u8>> {
    let input = input_blob(protected)?;
    let entropy = input_blob(entropy)?;
    let mut output = OutputBlob::empty();
    // SAFETY: as in `protect`. The description out-pointer may be null, so DPAPI allocates none.
    let succeeded = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            &entropy,
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output.0,
        )
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(output.bytes().to_vec())
}

/// Where an empty input points: DPAPI rejects a null pointer even for zero bytes, so .NET points
/// at a live dummy buffer instead (`s_nonEmpty`).
static NON_EMPTY: [u8; 1] = [0];

/// A blob DPAPI reads. Its pointer is mutable only because the struct is shared with outputs.
fn input_blob(bytes: &[u8]) -> io::Result<CRYPT_INTEGER_BLOB> {
    let length = u32::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "the data is too large"))?;
    let data = if bytes.is_empty() {
        &NON_EMPTY[..]
    } else {
        bytes
    };
    Ok(CRYPT_INTEGER_BLOB {
        cbData: length,
        pbData: data.as_ptr().cast_mut(),
    })
}

/// A blob DPAPI allocated with `LocalAlloc`, wiped and freed on drop.
struct OutputBlob(CRYPT_INTEGER_BLOB);

impl OutputBlob {
    fn empty() -> Self {
        Self(CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        })
    }

    fn bytes(&self) -> &[u8] {
        if self.0.pbData.is_null() {
            return &[];
        }
        // SAFETY: after a successful call DPAPI points `pbData` at `cbData` initialized bytes it
        // owns until `LocalFree`, which only `drop` calls. A null pointer was handled above, since
        // an empty secret may come back without a buffer.
        unsafe { std::slice::from_raw_parts(self.0.pbData, self.0.cbData as usize) }
    }
}

impl Drop for OutputBlob {
    fn drop(&mut self) {
        if self.0.pbData.is_null() {
            return;
        }
        // SAFETY: `pbData` is the live `LocalAlloc` buffer of `cbData` bytes DPAPI returned, freed
        // exactly once here. It may hold the clear secret, so it is zeroed first.
        unsafe {
            ptr::write_bytes(self.0.pbData, 0, self.0.cbData as usize);
            LocalFree(self.0.pbData.cast());
        }
    }
}

#[cfg(test)]
mod tests;
