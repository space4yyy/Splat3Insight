//! Cross-platform account and current-battle state for the desktop app.
//!
//! This crate owns account data and the live SplatNet data boundary used by
//! the desktop app.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub mod history;
pub mod monitor;
pub mod nintendo_auth;
pub mod overlay;
pub mod splatnet_auth;

pub const KEYRING_SERVICE: &str = "ink.space4.splat3streamingwidget";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountMetadata {
    pub account_id: String,
    pub display_name: String,
    pub language: Option<String>,
    pub country: Option<String>,
    pub created_at_ms: i64,
    pub last_used_at_ms: i64,
    pub baseline_at_ms: Option<i64>,
    pub last_seen_battle_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountSecret {
    pub session_token: String,
    pub game_web_token: Option<String>,
    pub bullet_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct RegistryFile {
    active_account_id: Option<String>,
    accounts: Vec<AccountMetadata>,
    #[serde(default)]
    pending_account_deletions: Vec<String>,
}

pub trait SecretStore {
    fn put(&self, account_id: &str, secret: &AccountSecret) -> Result<(), String>;
    fn get(&self, account_id: &str) -> Result<Option<AccountSecret>, String>;
    fn delete(&self, account_id: &str) -> Result<(), String>;
}

/// Native OS credential storage.
///
/// The account id is part of the credential username, so multiple Nintendo
/// accounts can coexist without overwriting each other.
#[derive(Debug, Clone)]
pub struct KeyringSecretStore {
    service: String,
}

impl Default for KeyringSecretStore {
    fn default() -> Self {
        Self::new(KEYRING_SERVICE)
    }
}

impl KeyringSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, account_id: &str) -> Result<keyring::Entry, String> {
        let username = format!("nintendo:{account_id}");
        keyring::Entry::new(&self.service, &username).map_err(|error| error.to_string())
    }
}

impl SecretStore for KeyringSecretStore {
    fn put(&self, account_id: &str, secret: &AccountSecret) -> Result<(), String> {
        let entry = self.entry(account_id)?;
        let encoded = serde_json::to_string(secret).map_err(|error| error.to_string())?;
        entry
            .set_password(&encoded)
            .map_err(|error| error.to_string())
    }

    fn get(&self, account_id: &str) -> Result<Option<AccountSecret>, String> {
        let entry = self.entry(account_id)?;
        match entry.get_password() {
            Ok(encoded) => serde_json::from_str(&encoded)
                .map(Some)
                .map_err(|error| error.to_string()),
            Err(error) if is_missing_credential(&error) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn delete(&self, account_id: &str) -> Result<(), String> {
        let entry = self.entry(account_id)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(error) if is_missing_credential(&error) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

fn is_missing_credential(error: &keyring::Error) -> bool {
    matches!(error, keyring::Error::NoEntry)
}

/// Stores non-sensitive account metadata on disk and delegates all secrets to
/// a SecretStore. The metadata file never contains session or web tokens.
pub struct AccountRegistry<S> {
    metadata_path: PathBuf,
    secret_store: S,
    file: RegistryFile,
}

impl<S: SecretStore> AccountRegistry<S> {
    pub fn load(metadata_path: impl Into<PathBuf>, secret_store: S) -> Result<Self, String> {
        let metadata_path = metadata_path.into();
        let file = if metadata_path.exists() {
            let contents = fs::read_to_string(&metadata_path).map_err(|error| error.to_string())?;
            serde_json::from_str(&contents).map_err(|error| error.to_string())?
        } else {
            RegistryFile::default()
        };

        Ok(Self {
            metadata_path,
            secret_store,
            file,
        })
    }

    pub fn accounts(&self) -> &[AccountMetadata] {
        &self.file.accounts
    }

    pub fn active_account_id(&self) -> Option<&str> {
        self.file.active_account_id.as_deref()
    }

    pub fn active_account(&self) -> Option<&AccountMetadata> {
        let active_id = self.active_account_id()?;
        self.file
            .accounts
            .iter()
            .find(|account| account.account_id == active_id)
    }

    pub fn get_secret(&self, account_id: &str) -> Result<Option<AccountSecret>, String> {
        self.secret_store.get(account_id)
    }

    pub fn pending_account_deletions(&self) -> &[String] {
        &self.file.pending_account_deletions
    }

    pub fn is_account_deletion_pending(&self, account_id: &str) -> bool {
        self.file
            .pending_account_deletions
            .iter()
            .any(|pending| pending == account_id)
    }

    pub fn mark_account_deletion_pending(&mut self, account_id: &str) -> Result<(), String> {
        if !self.is_account_deletion_pending(account_id) {
            self.file
                .pending_account_deletions
                .push(account_id.to_owned());
            if let Err(error) = self.persist() {
                self.file
                    .pending_account_deletions
                    .retain(|pending| pending != account_id);
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn clear_account_deletion_pending(&mut self, account_id: &str) -> Result<(), String> {
        let previous = self.file.pending_account_deletions.clone();
        self.file
            .pending_account_deletions
            .retain(|pending| pending != account_id);
        if let Err(error) = self.persist() {
            self.file.pending_account_deletions = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn upsert(
        &mut self,
        metadata: AccountMetadata,
        secret: AccountSecret,
    ) -> Result<(), String> {
        if self.is_account_deletion_pending(&metadata.account_id) {
            return Err("该账号正在完成删除，无法更新登录信息".to_owned());
        }
        self.secret_store.put(&metadata.account_id, &secret)?;

        if let Some(existing) = self
            .file
            .accounts
            .iter_mut()
            .find(|account| account.account_id == metadata.account_id)
        {
            *existing = metadata;
        } else {
            self.file.accounts.push(metadata);
        }

        self.file.accounts.sort_by(|left, right| {
            left.display_name
                .to_lowercase()
                .cmp(&right.display_name.to_lowercase())
        });
        self.persist()
    }

    pub fn set_active(&mut self, account_id: &str, used_at_ms: i64) -> Result<(), String> {
        if self.is_account_deletion_pending(account_id) {
            return Err("该账号正在完成删除，无法切换使用".to_owned());
        }
        let account = self
            .file
            .accounts
            .iter_mut()
            .find(|account| account.account_id == account_id)
            .ok_or_else(|| format!("unknown account: {account_id}"))?;
        account.last_used_at_ms = used_at_ms;
        self.file.active_account_id = Some(account_id.to_owned());
        self.persist()
    }

    pub fn update_cursor(
        &mut self,
        account_id: &str,
        baseline_at_ms: i64,
        last_seen_battle_id: Option<String>,
    ) -> Result<(), String> {
        let account = self
            .file
            .accounts
            .iter_mut()
            .find(|account| account.account_id == account_id)
            .ok_or_else(|| format!("unknown account: {account_id}"))?;
        account.baseline_at_ms = Some(baseline_at_ms);
        account.last_seen_battle_id = last_seen_battle_id;
        self.persist()
    }

    pub fn remove(&mut self, account_id: &str) -> Result<(), String> {
        self.secret_store.delete(account_id)?;
        self.file
            .accounts
            .retain(|account| account.account_id != account_id);
        if self.file.active_account_id.as_deref() == Some(account_id) {
            self.file.active_account_id = None;
        }
        self.persist()
    }

    fn persist(&self) -> Result<(), String> {
        if let Some(parent) = self.metadata_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let encoded =
            serde_json::to_string_pretty(&self.file).map_err(|error| error.to_string())?;
        fs::write(&self.metadata_path, format!("{encoded}\n")).map_err(|error| error.to_string())
    }
}

/// In-memory credential store used by unit tests and the development preview.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    secrets: std::sync::Mutex<HashMap<String, AccountSecret>>,
}

impl SecretStore for MemorySecretStore {
    fn put(&self, account_id: &str, secret: &AccountSecret) -> Result<(), String> {
        self.secrets
            .lock()
            .map_err(|_| "memory secret store poisoned".to_owned())?
            .insert(account_id.to_owned(), secret.clone());
        Ok(())
    }

    fn get(&self, account_id: &str) -> Result<Option<AccountSecret>, String> {
        Ok(self
            .secrets
            .lock()
            .map_err(|_| "memory secret store poisoned".to_owned())?
            .get(account_id)
            .cloned())
    }

    fn delete(&self, account_id: &str) -> Result<(), String> {
        self.secrets
            .lock()
            .map_err(|_| "memory secret store poisoned".to_owned())?
            .remove(account_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn keyring_service_matches_the_configured_bundle_identifier() {
        assert_eq!(KEYRING_SERVICE, "ink.space4.splat3streamingwidget");
    }

    fn metadata(id: &str, name: &str) -> AccountMetadata {
        AccountMetadata {
            account_id: id.to_owned(),
            display_name: name.to_owned(),
            language: Some("zh-CN".to_owned()),
            country: Some("JP".to_owned()),
            created_at_ms: 1,
            last_used_at_ms: 1,
            baseline_at_ms: None,
            last_seen_battle_id: None,
        }
    }

    fn secret(value: &str) -> AccountSecret {
        AccountSecret {
            session_token: format!("session-{value}"),
            game_web_token: Some(format!("game-{value}")),
            bullet_token: Some(format!("bullet-{value}")),
        }
    }

    #[test]
    fn keeps_multiple_accounts_and_secrets_separate() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("accounts.json");
        let store = MemorySecretStore::default();
        let mut registry = AccountRegistry::load(&path, store).unwrap();

        registry
            .upsert(metadata("a", "Alice"), secret("a"))
            .unwrap();
        registry.upsert(metadata("b", "Bob"), secret("b")).unwrap();

        assert_eq!(registry.accounts().len(), 2);
        assert_eq!(registry.get_secret("a").unwrap(), Some(secret("a")));
        assert_eq!(registry.get_secret("b").unwrap(), Some(secret("b")));
        assert_eq!(registry.get_secret("missing").unwrap(), None);
    }

    #[test]
    fn persists_metadata_without_secrets() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("nested").join("accounts.json");
        let mut registry = AccountRegistry::load(&path, MemorySecretStore::default()).unwrap();
        registry
            .upsert(metadata("a", "Alice"), secret("sensitive"))
            .unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("Alice"));
        assert!(!contents.contains("sensitive"));
        assert!(!contents.contains("session-a"));
    }

    #[test]
    fn removes_active_account_and_credential() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("accounts.json");
        let store = MemorySecretStore::default();
        let mut registry = AccountRegistry::load(&path, store).unwrap();
        registry
            .upsert(metadata("a", "Alice"), secret("a"))
            .unwrap();
        registry.set_active("a", 10).unwrap();
        registry.remove("a").unwrap();

        assert!(registry.accounts().is_empty());
        assert_eq!(registry.active_account_id(), None);
        assert_eq!(registry.get_secret("a").unwrap(), None);
    }

    #[test]
    fn pending_account_deletion_survives_restart_until_explicitly_cleared() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("accounts.json");
        let mut registry = AccountRegistry::load(&path, MemorySecretStore::default()).unwrap();
        registry.mark_account_deletion_pending("a").unwrap();

        let mut reopened = AccountRegistry::load(&path, MemorySecretStore::default()).unwrap();
        assert!(reopened.is_account_deletion_pending("a"));
        assert_eq!(reopened.pending_account_deletions(), &["a"]);

        reopened.clear_account_deletion_pending("a").unwrap();
        assert!(!reopened.is_account_deletion_pending("a"));
    }

    #[test]
    fn pending_account_deletion_cannot_be_resurrected_by_token_refresh() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("accounts.json");
        let mut registry = AccountRegistry::load(&path, MemorySecretStore::default()).unwrap();
        registry.mark_account_deletion_pending("a").unwrap();

        assert!(registry
            .upsert(metadata("a", "Alice"), secret("a"))
            .is_err());
        assert_eq!(registry.get_secret("a").unwrap(), None);
        assert!(!path.exists() || !fs::read_to_string(path).unwrap().contains("Alice"));
    }
}
