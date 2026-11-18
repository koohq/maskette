//! Where API keys live: the OS credential store, never a plain file.
//!
//! Windows Credential Manager, macOS Keychain, or the Secret Service on
//! Linux, through the `keyring` crate. Each key is one entry, named after
//! its service (for example `api_key:openrouter.ai`) under [`SERVICE`]; see
//! [`super::store`]. Tests swap in [`MemorySecrets`].

#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::Mutex;

/// The service name the entries are filed under (the app identifier).
pub const SERVICE: &str = "app.maskette";

pub trait SecretStore: Send + Sync {
    /// `Ok(None)` when no secret is stored under `name`.
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    fn set(&self, name: &str, value: &str) -> Result<(), String>;
    /// Deleting a secret that is not there is not an error.
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// The OS credential store.
pub struct KeyringSecrets {
    service: String,
}

impl KeyringSecrets {
    pub fn new(service: &str) -> Self {
        Self {
            service: service.to_owned(),
        }
    }

    fn entry(&self, name: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(&self.service, name).map_err(|error| error.to_string())
    }
}

impl SecretStore for KeyringSecrets {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        match self.entry(name)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        self.entry(name)?
            .set_password(value)
            .map_err(|error| error.to_string())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Secrets kept in memory only, for tests.
#[cfg(test)]
#[derive(Default)]
pub struct MemorySecrets {
    entries: Mutex<BTreeMap<String, String>>,
    /// When set, every call fails with this message.
    pub fail_with: Mutex<Option<String>>,
    /// When set, storing and deleting fail with this message.
    pub fail_writes_with: Mutex<Option<String>>,
}

#[cfg(test)]
impl MemorySecrets {
    fn entries(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, String>>, String> {
        if let Some(message) = self.fail_with.lock().unwrap().clone() {
            return Err(message);
        }
        Ok(self.entries.lock().unwrap())
    }

    fn writable_entries(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, String>>, String> {
        if let Some(message) = self.fail_writes_with.lock().unwrap().clone() {
            return Err(message);
        }
        self.entries()
    }

    pub fn snapshot(&self) -> BTreeMap<String, String> {
        self.entries.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl SecretStore for MemorySecrets {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.entries()?.get(name).cloned())
    }

    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        self.writable_entries()?
            .insert(name.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        self.writable_entries()?.remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Touches the real credential store, so it only runs on request:
    /// `cargo test -- --ignored keyring_round_trip`.
    #[test]
    #[ignore = "uses the OS credential store"]
    fn keyring_round_trip() {
        let secrets = KeyringSecrets::new("app.maskette.test");
        let name = "round_trip_test_key";
        secrets.delete(name).unwrap();
        assert_eq!(secrets.get(name).unwrap(), None);
        secrets.set(name, "sk-test-0000").unwrap();
        assert_eq!(secrets.get(name).unwrap().as_deref(), Some("sk-test-0000"));
        secrets.delete(name).unwrap();
        secrets.delete(name).unwrap();
        assert_eq!(secrets.get(name).unwrap(), None);
    }
}
