//! Secrets such as the ActivityWatch token, ported from the `ISecretStore` port in
//! `IPersistencePorts.cs` and from `Secrets/DpapiSecretStore.cs`.

#[cfg(windows)]
mod dpapi;

use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::io;
use std::sync::{Mutex, PoisonError};

use sha2::{Digest, Sha256};

#[cfg(windows)]
pub use dpapi::DpapiSecretStore;

/// The name a secret is stored under. Never blank.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SecretId(String);

impl SecretId {
    /// Fails when `identifier` is empty or only white space, which .NET rejects too.
    pub fn new(identifier: impl Into<String>) -> Result<Self, BlankSecretId> {
        let identifier = identifier.into();
        if identifier.chars().all(char::is_whitespace) {
            return Err(BlankSecretId);
        }
        Ok(Self(identifier))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `<SHA-256 of the UTF-8 identifier, upper-case hex>.secret`, so that any identifier, even
    /// one that looks like a path, names one file directly inside the secrets directory.
    #[cfg_attr(
        not(windows),
        allow(dead_code, reason = "only DPAPI uses it outside the tests")
    )]
    fn file_name(&self) -> String {
        let mut name = String::with_capacity(71);
        for byte in Sha256::digest(self.0.as_bytes()) {
            let _ = write!(name, "{byte:02X}");
        }
        name.push_str(".secret");
        name
    }

    /// SHA-256 of `WorkTime:<identifier>` in UTF-8. Protecting with it ties a ciphertext to its
    /// identifier: a file copied to another identifier's name cannot be read.
    #[cfg_attr(
        not(windows),
        allow(dead_code, reason = "only DPAPI uses it outside the tests")
    )]
    fn entropy(&self) -> [u8; 32] {
        Sha256::digest(format!("WorkTime:{}", self.0)).into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlankSecretId;

impl fmt::Display for BlankSecretId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a secret identifier cannot be blank")
    }
}

impl std::error::Error for BlankSecretId {}

/// Keeps secrets by identifier. Mirrors `ISecretStore`, synchronously.
pub trait SecretStore {
    /// The secret saved under `id`, or `None` when there is none.
    fn get(&self, id: &SecretId) -> Result<Option<String>, SecretError>;

    /// Saves `secret` under `id`, replacing any previous one.
    fn save(&self, id: &SecretId, secret: &str) -> Result<(), SecretError>;

    /// Removes the secret saved under `id`. Removing a missing secret succeeds.
    fn delete(&self, id: &SecretId) -> Result<(), SecretError>;
}

impl<T: SecretStore + ?Sized> SecretStore for &T {
    fn get(&self, id: &SecretId) -> Result<Option<String>, SecretError> {
        (**self).get(id)
    }

    fn save(&self, id: &SecretId, secret: &str) -> Result<(), SecretError> {
        (**self).save(id, secret)
    }

    fn delete(&self, id: &SecretId) -> Result<(), SecretError> {
        (**self).delete(id)
    }
}

/// No error carries the secret or its ciphertext.
#[derive(Debug)]
pub enum SecretError {
    Io(io::Error),
    /// The operating system refused to encrypt the secret.
    Protect(io::Error),
    /// The stored secret could not be decrypted: the file is damaged, was written for another
    /// identifier, or belongs to another Windows user. The file is left as it was.
    Unprotect(io::Error),
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "the secret could not be read or written: {error}"),
            Self::Protect(error) => write!(f, "the secret could not be encrypted: {error}"),
            Self::Unprotect(error) => {
                write!(f, "the stored secret could not be decrypted: {error}")
            }
        }
    }
}

impl std::error::Error for SecretError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) | Self::Protect(error) | Self::Unprotect(error) => Some(error),
        }
    }
}

/// Secrets kept in memory, for tests and platforms without DPAPI.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    secrets: Mutex<HashMap<SecretId, String>>,
}

impl MemorySecretStore {
    fn secrets(&self) -> std::sync::MutexGuard<'_, HashMap<SecretId, String>> {
        self.secrets.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, id: &SecretId) -> Result<Option<String>, SecretError> {
        Ok(self.secrets().get(id).cloned())
    }

    fn save(&self, id: &SecretId, secret: &str) -> Result<(), SecretError> {
        self.secrets().insert(id.clone(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, id: &SecretId) -> Result<(), SecretError> {
        self.secrets().remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
