use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::CliError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AccountSecrets {
    ClientCredentials {
        client_id: String,
        client_secret: String,
        username: String,
    },
    AuthCode {
        client_id: String,
        client_secret: String,
        refresh_token: Option<String>,
    },
    /// Like `ClientCredentials`, but the client id/secret live in the named app's
    /// keychain entry instead of being copied here — rotating the app's secret
    /// updates every referencing account at once.
    ClientCredentialsApp { app: String, username: String },
    /// Like `AuthCode`, with the client id/secret resolved through the named app.
    AuthCodeApp {
        app: String,
        refresh_token: Option<String>,
    },
}

/// The secret half of a registered client application (`intacct-cli app add`);
/// the name → client-id mapping lives in the config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSecrets {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedToken {
    pub access_token: String,
    pub expires_at_epoch: u64,
}

impl CachedToken {
    const LEEWAY_SECONDS: u64 = 60;

    pub fn is_valid_at(&self, now_epoch: u64) -> bool {
        self.expires_at_epoch > now_epoch + Self::LEEWAY_SECONDS
    }
}

pub trait SecretStore: Send + Sync {
    fn get(&self, alias: &str) -> Result<Option<AccountSecrets>, CliError>;
    fn set(&self, alias: &str, secrets: &AccountSecrets) -> Result<(), CliError>;
    fn delete(&self, alias: &str) -> Result<(), CliError>;
    fn get_token(&self, alias: &str) -> Result<Option<CachedToken>, CliError>;
    fn set_token(&self, alias: &str, token: &CachedToken) -> Result<(), CliError>;
    fn delete_token(&self, alias: &str) -> Result<(), CliError>;
    fn get_app(&self, name: &str) -> Result<Option<AppSecrets>, CliError>;
    fn set_app(&self, name: &str, app: &AppSecrets) -> Result<(), CliError>;
    fn delete_app(&self, name: &str) -> Result<(), CliError>;
}

pub struct KeyringStore;

const KEYRING_SERVICE: &str = "intacct-cli";

impl KeyringStore {
    fn entry(user: &str) -> Result<keyring::Entry, CliError> {
        keyring::Entry::new(KEYRING_SERVICE, user).map_err(|keyring_error| {
            CliError::Auth(format!("keychain unavailable: {keyring_error}"))
        })
    }

    fn read<T: for<'de> Deserialize<'de>>(user: &str) -> Result<Option<T>, CliError> {
        match Self::entry(user)?.get_password() {
            Ok(raw) => serde_json::from_str(&raw).map(Some).map_err(|parse_error| {
                CliError::Auth(format!("corrupt keychain entry '{user}': {parse_error}"))
            }),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(keyring_error) => Err(CliError::Auth(format!(
                "keychain read failed: {keyring_error}"
            ))),
        }
    }

    fn write<T: Serialize>(user: &str, value: &T) -> Result<(), CliError> {
        Self::entry(user)?
            .set_password(&serde_json::to_string(value).expect("serializable"))
            .map_err(|keyring_error| {
                CliError::Auth(format!("keychain write failed: {keyring_error}"))
            })
    }

    fn remove(user: &str) -> Result<(), CliError> {
        match Self::entry(user)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(keyring_error) => Err(CliError::Auth(format!(
                "keychain delete failed: {keyring_error}"
            ))),
        }
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, alias: &str) -> Result<Option<AccountSecrets>, CliError> {
        KeyringStore::read(alias)
    }
    fn set(&self, alias: &str, secrets: &AccountSecrets) -> Result<(), CliError> {
        KeyringStore::write(alias, secrets)
    }
    fn delete(&self, alias: &str) -> Result<(), CliError> {
        KeyringStore::remove(alias)?;
        KeyringStore::remove(&format!("{alias}#token"))
    }
    fn get_token(&self, alias: &str) -> Result<Option<CachedToken>, CliError> {
        KeyringStore::read(&format!("{alias}#token"))
    }
    fn set_token(&self, alias: &str, token: &CachedToken) -> Result<(), CliError> {
        KeyringStore::write(&format!("{alias}#token"), token)
    }
    fn delete_token(&self, alias: &str) -> Result<(), CliError> {
        KeyringStore::remove(&format!("{alias}#token"))
    }
    fn get_app(&self, name: &str) -> Result<Option<AppSecrets>, CliError> {
        KeyringStore::read(&format!("app#{name}"))
    }
    fn set_app(&self, name: &str, app: &AppSecrets) -> Result<(), CliError> {
        KeyringStore::write(&format!("app#{name}"), app)
    }
    fn delete_app(&self, name: &str) -> Result<(), CliError> {
        KeyringStore::remove(&format!("app#{name}"))
    }
}

/// Wraps a [`SecretStore`] so app-referencing account entries behave exactly like inline
/// ones to the token providers: `get` materializes `*App` variants by pulling the client
/// id/secret from the app's entry, and `set` preserves the reference when a provider
/// writes back a materialized value (the auth-code provider persists rotated refresh
/// tokens as inline `AuthCode` — only the refresh token must survive that write).
pub struct ResolvingStore<Inner: SecretStore> {
    inner: Inner,
}

impl<Inner: SecretStore> ResolvingStore<Inner> {
    pub fn new(inner: Inner) -> Self {
        ResolvingStore { inner }
    }

    fn app_secrets(&self, name: &str) -> Result<AppSecrets, CliError> {
        self.inner.get_app(name)?.ok_or_else(|| {
            CliError::Auth(format!(
                "client app '{name}' has no stored secret; run `intacct-cli app add`"
            ))
        })
    }
}

impl<Inner: SecretStore> SecretStore for ResolvingStore<Inner> {
    fn get(&self, alias: &str) -> Result<Option<AccountSecrets>, CliError> {
        match self.inner.get(alias)? {
            Some(AccountSecrets::ClientCredentialsApp { app, username }) => {
                let creds = self.app_secrets(&app)?;
                Ok(Some(AccountSecrets::ClientCredentials {
                    client_id: creds.client_id,
                    client_secret: creds.client_secret,
                    username,
                }))
            }
            Some(AccountSecrets::AuthCodeApp { app, refresh_token }) => {
                let creds = self.app_secrets(&app)?;
                Ok(Some(AccountSecrets::AuthCode {
                    client_id: creds.client_id,
                    client_secret: creds.client_secret,
                    refresh_token,
                }))
            }
            other => Ok(other),
        }
    }

    fn set(&self, alias: &str, secrets: &AccountSecrets) -> Result<(), CliError> {
        let preserved = match (self.inner.get(alias)?, secrets) {
            (
                Some(AccountSecrets::AuthCodeApp { app, .. }),
                AccountSecrets::AuthCode { refresh_token, .. },
            ) => Some(AccountSecrets::AuthCodeApp {
                app,
                refresh_token: refresh_token.clone(),
            }),
            (
                Some(AccountSecrets::ClientCredentialsApp { app, .. }),
                AccountSecrets::ClientCredentials { username, .. },
            ) => Some(AccountSecrets::ClientCredentialsApp {
                app,
                username: username.clone(),
            }),
            _ => None,
        };
        self.inner.set(alias, preserved.as_ref().unwrap_or(secrets))
    }

    fn delete(&self, alias: &str) -> Result<(), CliError> {
        self.inner.delete(alias)
    }
    fn get_token(&self, alias: &str) -> Result<Option<CachedToken>, CliError> {
        self.inner.get_token(alias)
    }
    fn set_token(&self, alias: &str, token: &CachedToken) -> Result<(), CliError> {
        self.inner.set_token(alias, token)
    }
    fn delete_token(&self, alias: &str) -> Result<(), CliError> {
        self.inner.delete_token(alias)
    }
    fn get_app(&self, name: &str) -> Result<Option<AppSecrets>, CliError> {
        self.inner.get_app(name)
    }
    fn set_app(&self, name: &str, app: &AppSecrets) -> Result<(), CliError> {
        self.inner.set_app(name, app)
    }
    fn delete_app(&self, name: &str) -> Result<(), CliError> {
        self.inner.delete_app(name)
    }
}

#[derive(Default)]
pub struct MemoryStore {
    entries: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    fn read<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Result<Option<T>, CliError> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .get(key)
            .map(|raw| serde_json::from_str(raw).expect("valid stored json")))
    }

    fn write<T: Serialize>(&self, key: &str, value: &T) -> Result<(), CliError> {
        self.entries
            .lock()
            .unwrap()
            .insert(key.into(), serde_json::to_string(value).unwrap());
        Ok(())
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, alias: &str) -> Result<Option<AccountSecrets>, CliError> {
        self.read(alias)
    }
    fn set(&self, alias: &str, secrets: &AccountSecrets) -> Result<(), CliError> {
        self.write(alias, secrets)
    }
    fn delete(&self, alias: &str) -> Result<(), CliError> {
        let mut entries = self.entries.lock().unwrap();
        entries.remove(alias);
        entries.remove(&format!("{alias}#token"));
        Ok(())
    }
    fn get_token(&self, alias: &str) -> Result<Option<CachedToken>, CliError> {
        self.read(&format!("{alias}#token"))
    }
    fn set_token(&self, alias: &str, token: &CachedToken) -> Result<(), CliError> {
        self.write(&format!("{alias}#token"), token)
    }
    fn delete_token(&self, alias: &str) -> Result<(), CliError> {
        self.entries
            .lock()
            .unwrap()
            .remove(&format!("{alias}#token"));
        Ok(())
    }
    fn get_app(&self, name: &str) -> Result<Option<AppSecrets>, CliError> {
        self.read(&format!("app#{name}"))
    }
    fn set_app(&self, name: &str, app: &AppSecrets) -> Result<(), CliError> {
        self.write(&format!("app#{name}"), app)
    }
    fn delete_app(&self, name: &str) -> Result<(), CliError> {
        self.entries.lock().unwrap().remove(&format!("app#{name}"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trips_secrets_and_tokens() {
        let store = MemoryStore::default();
        let secrets = AccountSecrets::ClientCredentials {
            client_id: "cid.app.sage.com".into(),
            client_secret: "shhh".into(),
            username: "svc_api@creativeplanning".into(),
        };
        store.set("prod", &secrets).unwrap();
        match store.get("prod").unwrap().expect("stored") {
            AccountSecrets::ClientCredentials { username, .. } => {
                assert_eq!(username, "svc_api@creativeplanning")
            }
            other => panic!("wrong variant: {other:?}"),
        }
        assert!(store.get("absent").unwrap().is_none());

        let token = CachedToken {
            access_token: "tok".into(),
            expires_at_epoch: 999,
        };
        store.set_token("prod", &token).unwrap();
        assert_eq!(
            store.get_token("prod").unwrap().unwrap().access_token,
            "tok"
        );
        store.delete("prod").unwrap();
        assert!(store.get("prod").unwrap().is_none());
        assert!(store.get_token("prod").unwrap().is_none());
    }

    #[test]
    fn cached_token_expiry_check_uses_leeway() {
        let now = 1_000_000;
        let live = CachedToken {
            access_token: "a".into(),
            expires_at_epoch: now + 120,
        };
        let stale = CachedToken {
            access_token: "b".into(),
            expires_at_epoch: now + 10,
        };
        assert!(live.is_valid_at(now));
        assert!(!stale.is_valid_at(now));
    }

    #[test]
    fn auth_code_secrets_serialize_with_kebab_kind_tag() {
        let secrets = AccountSecrets::AuthCode {
            client_id: "cid".into(),
            client_secret: "cs".into(),
            refresh_token: Some("rt".into()),
        };
        let raw = serde_json::to_string(&secrets).unwrap();
        assert!(raw.contains(r#""kind":"auth-code""#), "got: {raw}");

        let app_ref = AccountSecrets::AuthCodeApp {
            app: "main".into(),
            refresh_token: None,
        };
        let raw = serde_json::to_string(&app_ref).unwrap();
        assert!(raw.contains(r#""kind":"auth-code-app""#), "got: {raw}");
    }

    fn store_with_app() -> ResolvingStore<MemoryStore> {
        let store = ResolvingStore::new(MemoryStore::default());
        store
            .set_app(
                "main",
                &AppSecrets {
                    client_id: "cid.app.sage.com".into(),
                    client_secret: "app-secret".into(),
                },
            )
            .unwrap();
        store
    }

    #[test]
    fn resolving_store_materializes_app_references_on_get() {
        let store = store_with_app();
        store
            .set(
                "acme",
                &AccountSecrets::AuthCodeApp {
                    app: "main".into(),
                    refresh_token: Some("rt".into()),
                },
            )
            .unwrap();
        match store.get("acme").unwrap().expect("stored") {
            AccountSecrets::AuthCode {
                client_id,
                client_secret,
                refresh_token,
            } => {
                assert_eq!(client_id, "cid.app.sage.com");
                assert_eq!(client_secret, "app-secret");
                assert_eq!(refresh_token.as_deref(), Some("rt"));
            }
            other => panic!("wrong variant: {other:?}"),
        }

        store
            .set(
                "acme-svc",
                &AccountSecrets::ClientCredentialsApp {
                    app: "main".into(),
                    username: "cli@acme".into(),
                },
            )
            .unwrap();
        match store.get("acme-svc").unwrap().expect("stored") {
            AccountSecrets::ClientCredentials {
                client_secret,
                username,
                ..
            } => {
                assert_eq!(client_secret, "app-secret");
                assert_eq!(username, "cli@acme");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn resolving_store_keeps_reference_when_provider_writes_back_inline() {
        let store = store_with_app();
        store
            .set(
                "acme",
                &AccountSecrets::AuthCodeApp {
                    app: "main".into(),
                    refresh_token: Some("old-rt".into()),
                },
            )
            .unwrap();
        // Simulate the auth-code provider persisting a rotated refresh token as a
        // materialized inline value.
        store
            .set(
                "acme",
                &AccountSecrets::AuthCode {
                    client_id: "cid.app.sage.com".into(),
                    client_secret: "app-secret".into(),
                    refresh_token: Some("rotated-rt".into()),
                },
            )
            .unwrap();
        // The underlying entry must still be an app reference with the new token.
        match store.inner.get("acme").unwrap().expect("stored") {
            AccountSecrets::AuthCodeApp { app, refresh_token } => {
                assert_eq!(app, "main");
                assert_eq!(refresh_token.as_deref(), Some("rotated-rt"));
            }
            other => panic!("reference was lost: {other:?}"),
        }
        // A later secret rotation on the app is picked up on the next get.
        store
            .set_app(
                "main",
                &AppSecrets {
                    client_id: "cid.app.sage.com".into(),
                    client_secret: "rotated-secret".into(),
                },
            )
            .unwrap();
        match store.get("acme").unwrap().expect("stored") {
            AccountSecrets::AuthCode { client_secret, .. } => {
                assert_eq!(client_secret, "rotated-secret")
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn resolving_store_errors_clearly_when_app_secret_is_missing() {
        let store = ResolvingStore::new(MemoryStore::default());
        store
            .set(
                "orphan",
                &AccountSecrets::AuthCodeApp {
                    app: "ghost".into(),
                    refresh_token: None,
                },
            )
            .unwrap();
        let error = store.get("orphan").unwrap_err();
        assert!(
            error.to_string().contains("ghost"),
            "error should name the missing app: {error}"
        );
    }
}
