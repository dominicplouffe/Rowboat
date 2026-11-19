//! Secrets: database passwords, SSH passwords and key passphrases.
//!
//! They live in the operating system's keychain (macOS Keychain, Windows
//! Credential Manager, Linux Secret Service) and nowhere else: never in the
//! settings file, the connections database, logs or exports. The UI sends a
//! secret once when the user types it and can never read it back; it only
//! learns whether one is stored.
//!
//! If the machine has no usable keychain (a headless Linux box without a
//! Secret Service, for example) secrets are kept in memory for this run only,
//! and the UI says so. There is deliberately no file fallback.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

const SERVICE: &str = "com.dominicplouffe.rowboat";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretKind {
    Password,
    SshPassword,
    SshPassphrase,
}

impl SecretKind {
    pub const ALL: [SecretKind; 3] = [Self::Password, Self::SshPassword, Self::SshPassphrase];

    fn slug(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::SshPassword => "ssh-password",
            Self::SshPassphrase => "ssh-passphrase",
        }
    }
}

/// Where secrets are kept. A trait so tests never touch the real keychain.
pub trait Backend: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// The operating system's keychain.
pub struct Keychain;

impl Backend for Keychain {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new(SERVICE, account).and_then(|entry| entry.get_password()) {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, account)
            .and_then(|entry| entry.set_password(secret))
            .map_err(|e| e.to_string())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        match keyring::Entry::new(SERVICE, account).and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// In-memory storage: for tests, and for this run when there is no keychain.
#[derive(Default)]
pub struct Memory(Mutex<HashMap<String, String>>);

impl Backend for Memory {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(account)
            .cloned())
    }
    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(account.to_string(), secret.to_string());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(account);
        Ok(())
    }
}

pub struct Secrets {
    /// Chosen on first use: probing the keychain can take a moment, and it
    /// must never be part of app startup.
    chosen: OnceLock<(Box<dyn Backend>, bool)>,
    probe: fn() -> (Box<dyn Backend>, bool),
}

fn probe_system() -> (Box<dyn Backend>, bool) {
    let keychain = Keychain;
    // A harmless round trip tells us whether a keychain is really there.
    let probe = "rowboat-keychain-probe";
    if keychain.set(probe, "ok").is_ok() && keychain.delete(probe).is_ok() {
        (Box::new(keychain), true)
    } else {
        (Box::new(Memory::default()), false)
    }
}

impl Secrets {
    /// Uses the OS keychain when it works, memory otherwise.
    pub fn system() -> Self {
        Self {
            chosen: OnceLock::new(),
            probe: probe_system,
        }
    }

    pub fn in_memory() -> Self {
        Self {
            chosen: OnceLock::new(),
            probe: || (Box::new(Memory::default()), false),
        }
    }

    pub fn with_backend(backend: Box<dyn Backend>) -> Self {
        let secrets = Self {
            chosen: OnceLock::new(),
            probe: || unreachable!("the backend was given"),
        };
        let _ = secrets.chosen.set((backend, true));
        secrets
    }

    fn backend(&self) -> &dyn Backend {
        self.chosen.get_or_init(self.probe).0.as_ref()
    }

    /// False when secrets only last until the app quits.
    pub fn is_persistent(&self) -> bool {
        self.chosen.get_or_init(self.probe).1
    }

    pub(crate) fn account(connection_id: &str, kind: SecretKind) -> String {
        format!("connection/{connection_id}/{}", kind.slug())
    }

    pub fn get(&self, connection_id: &str, kind: SecretKind) -> Result<Option<String>, String> {
        self.backend().get(&Self::account(connection_id, kind))
    }

    /// Stores a secret; an empty one deletes it.
    pub fn set(&self, connection_id: &str, kind: SecretKind, secret: &str) -> Result<(), String> {
        let account = Self::account(connection_id, kind);
        if secret.is_empty() {
            self.backend().delete(&account)
        } else {
            self.backend().set(&account, secret)
        }
    }

    #[cfg(test)]
    pub fn has(&self, connection_id: &str, kind: SecretKind) -> bool {
        matches!(self.get(connection_id, kind), Ok(Some(_)))
    }

    pub(crate) fn ai_account(provider_id: &str) -> String {
        format!("ai/{provider_id}/key")
    }

    pub fn ai_key(&self, provider_id: &str) -> Result<Option<String>, String> {
        self.backend().get(&Self::ai_account(provider_id))
    }

    /// Stores an AI provider's API key; an empty one deletes it.
    pub fn set_ai_key(&self, provider_id: &str, key: &str) -> Result<(), String> {
        let account = Self::ai_account(provider_id);
        if key.is_empty() {
            self.backend().delete(&account)
        } else {
            self.backend().set(&account, key)
        }
    }

    /// Removes every secret of a deleted connection.
    pub fn forget(&self, connection_id: &str) {
        for kind in SecretKind::ALL {
            let _ = self.backend().delete(&Self::account(connection_id, kind));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_kept_per_connection_and_kind() {
        let secrets = Secrets::in_memory();
        secrets.set("a", SecretKind::Password, "pw-a").unwrap();
        secrets
            .set("a", SecretKind::SshPassphrase, "phrase")
            .unwrap();
        secrets.set("b", SecretKind::Password, "pw-b").unwrap();

        assert_eq!(
            secrets.get("a", SecretKind::Password).unwrap().as_deref(),
            Some("pw-a")
        );
        assert_eq!(
            secrets.get("b", SecretKind::Password).unwrap().as_deref(),
            Some("pw-b")
        );
        assert_eq!(secrets.get("a", SecretKind::SshPassword).unwrap(), None);
        assert!(secrets.has("a", SecretKind::SshPassphrase));
    }

    #[test]
    fn an_empty_secret_deletes_the_stored_one() {
        let secrets = Secrets::in_memory();
        secrets.set("a", SecretKind::Password, "pw").unwrap();
        secrets.set("a", SecretKind::Password, "").unwrap();
        assert!(!secrets.has("a", SecretKind::Password));
    }

    #[test]
    fn forgetting_a_connection_removes_all_its_secrets_only() {
        let secrets = Secrets::in_memory();
        for kind in SecretKind::ALL {
            secrets.set("gone", kind, "x").unwrap();
        }
        secrets.set("kept", SecretKind::Password, "y").unwrap();
        secrets.forget("gone");
        assert!(SecretKind::ALL.iter().all(|&k| !secrets.has("gone", k)));
        assert!(secrets.has("kept", SecretKind::Password));
    }

    #[test]
    fn memory_storage_admits_it_is_not_persistent() {
        assert!(!Secrets::in_memory().is_persistent());
        assert!(Secrets::with_backend(Box::new(Memory::default())).is_persistent());
    }
}
