use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::error::{AuthError, AuthResult};

const SERVICE: &str = "chhanni";

/// Abstraction over a secret store so the OAuth code can be tested without
/// touching the real OS keychain.
pub trait SecretStore: Send + Sync + 'static {
    fn get(&self, key: &str) -> AuthResult<Option<String>>;
    fn set(&self, key: &str, value: &str) -> AuthResult<()>;
    fn delete(&self, key: &str) -> AuthResult<()>;
}

/// Production implementation backed by the OS keychain.
#[derive(Clone, Default)]
pub struct Keychain;

impl Keychain {
    pub fn new() -> Self {
        Self
    }
}

impl SecretStore for Keychain {
    fn get(&self, key: &str) -> AuthResult<Option<String>> {
        let entry = keyring::Entry::new(SERVICE, key)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::Keychain(e)),
        }
    }

    fn set(&self, key: &str, value: &str) -> AuthResult<()> {
        let entry = keyring::Entry::new(SERVICE, key)?;
        entry.set_password(value)?;
        Ok(())
    }

    fn delete(&self, key: &str) -> AuthResult<()> {
        let entry = keyring::Entry::new(SERVICE, key)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AuthError::Keychain(e)),
        }
    }
}

/// In-memory `SecretStore` used in tests. Shareable across threads.
#[derive(Clone, Default)]
pub struct MemoryStore {
    inner: Arc<Mutex<HashMap<String, String>>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().is_empty()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> AuthResult<Option<String>> {
        Ok(self.inner.lock().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> AuthResult<()> {
        self.inner.lock().insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> AuthResult<()> {
        self.inner.lock().remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trips() {
        let store = MemoryStore::new();
        assert!(store.get("k").unwrap().is_none());
        store.set("k", "v").unwrap();
        assert_eq!(store.get("k").unwrap().as_deref(), Some("v"));
        store.delete("k").unwrap();
        assert!(store.get("k").unwrap().is_none());
    }

    #[test]
    fn memory_store_overwrite() {
        let store = MemoryStore::new();
        store.set("k", "v1").unwrap();
        store.set("k", "v2").unwrap();
        assert_eq!(store.get("k").unwrap().as_deref(), Some("v2"));
    }

    #[test]
    fn memory_store_delete_missing_is_ok() {
        let store = MemoryStore::new();
        store.delete("never-set").unwrap();
    }
}
