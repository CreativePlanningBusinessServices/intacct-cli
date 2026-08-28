use std::path::Path;

use serde_json::{Value, json};

use crate::config::{AppEntry, Config};
use crate::error::CliError;
use crate::secrets::{AppSecrets, SecretStore};

pub fn add(
    config_path: &Path,
    store: &dyn SecretStore,
    name: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<Value, CliError> {
    let mut config = Config::load(config_path)?;
    config.apps.insert(
        name.to_string(),
        AppEntry {
            client_id: client_id.to_string(),
        },
    );
    if config.default_app.is_none() {
        config.default_app = Some(name.to_string());
    }
    let is_default = config.default_app.as_deref() == Some(name);
    // Same ordering rationale as account add: config first, so a failed keychain write
    // leaves a self-describing "no stored secret; run app add" state instead of an
    // orphaned keychain entry.
    config.save(config_path)?;
    store.set_app(
        name,
        &AppSecrets {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
        },
    )?;
    Ok(json!({
        "app": name,
        "clientId": client_id,
        "default": is_default,
    }))
}

pub fn list(config_path: &Path) -> Result<Value, CliError> {
    let config = Config::load(config_path)?;
    let apps: Vec<Value> = config
        .apps
        .iter()
        .map(|(name, entry)| {
            json!({
                "app": name,
                "clientId": entry.client_id,
                "default": config.default_app.as_deref() == Some(name.as_str()),
                "accounts": config
                    .accounts
                    .values()
                    .filter(|account| account.app.as_deref() == Some(name.as_str()))
                    .count(),
            })
        })
        .collect();
    Ok(json!({"count": apps.len(), "apps": apps}))
}

/// Rotation entry point: re-stores the secret (and optionally a new client id) under the
/// same name, which every referencing account picks up on its next token request.
pub fn update(
    config_path: &Path,
    store: &dyn SecretStore,
    name: &str,
    client_id: Option<&str>,
    client_secret: &str,
) -> Result<Value, CliError> {
    let mut config = Config::load(config_path)?;
    let entry = config.apps.get_mut(name).ok_or_else(|| {
        CliError::Usage(format!("unknown client app '{name}'; run `intacct-cli app list`"))
    })?;
    if let Some(new_client_id) = client_id {
        entry.client_id = new_client_id.to_string();
    }
    let effective_client_id = entry.client_id.clone();
    config.save(config_path)?;
    store.set_app(
        name,
        &AppSecrets {
            client_id: effective_client_id.clone(),
            client_secret: client_secret.to_string(),
        },
    )?;
    Ok(json!({"app": name, "clientId": effective_client_id, "updated": true}))
}

pub fn set_default(config_path: &Path, name: &str) -> Result<Value, CliError> {
    let mut config = Config::load(config_path)?;
    if !config.apps.contains_key(name) {
        return Err(CliError::Usage(format!(
            "unknown client app '{name}'; run `intacct-cli app list`"
        )));
    }
    config.default_app = Some(name.to_string());
    config.save(config_path)?;
    Ok(json!({"defaultApp": name}))
}

pub fn remove(config_path: &Path, store: &dyn SecretStore, name: &str) -> Result<Value, CliError> {
    let mut config = Config::load(config_path)?;
    if !config.apps.contains_key(name) {
        return Err(CliError::Usage(format!(
            "unknown client app '{name}'; run `intacct-cli app list`"
        )));
    }
    let referencing: Vec<&String> = config
        .accounts
        .iter()
        .filter(|(_, account)| account.app.as_deref() == Some(name))
        .map(|(alias, _)| alias)
        .collect();
    if !referencing.is_empty() {
        return Err(CliError::Usage(format!(
            "client app '{name}' is used by account(s) {}; remove them first",
            referencing
                .iter()
                .map(|alias| format!("'{alias}'"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    config.apps.remove(name);
    if config.default_app.as_deref() == Some(name) {
        config.default_app = None;
    }
    config.save(config_path)?;
    store.delete_app(name)?;
    Ok(json!({"removed": name}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AccountEntry, AuthFlow};
    use crate::secrets::MemoryStore;

    fn temp_config() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        (dir, path)
    }

    #[test]
    fn first_app_added_becomes_default_and_stores_secret() {
        let (_dir, path) = temp_config();
        let store = MemoryStore::default();
        let result = add(&path, &store, "main", "cid.app.sage.com", "shhh").unwrap();
        assert_eq!(result["default"], true);
        assert_eq!(
            store.get_app("main").unwrap().unwrap().client_secret,
            "shhh"
        );

        let second = add(&path, &store, "other", "cid2.app.sage.com", "shhh2").unwrap();
        assert_eq!(second["default"], false);

        let listed = list(&path).unwrap();
        assert_eq!(listed["count"], 2);
    }

    #[test]
    fn update_rotates_secret_and_keeps_client_id_unless_given() {
        let (_dir, path) = temp_config();
        let store = MemoryStore::default();
        add(&path, &store, "main", "cid.app.sage.com", "old").unwrap();
        update(&path, &store, "main", None, "new").unwrap();
        let app = store.get_app("main").unwrap().unwrap();
        assert_eq!(app.client_id, "cid.app.sage.com");
        assert_eq!(app.client_secret, "new");
        assert!(matches!(
            update(&path, &store, "ghost", None, "x"),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn remove_refuses_while_accounts_reference_the_app() {
        let (_dir, path) = temp_config();
        let store = MemoryStore::default();
        add(&path, &store, "main", "cid.app.sage.com", "shhh").unwrap();

        let mut config = Config::load(&path).unwrap();
        config.accounts.insert(
            "acme".into(),
            AccountEntry {
                company_id: "acme".into(),
                user_id: None,
                entity_id: None,
                app: Some("main".into()),
                flow: AuthFlow::AuthCode,
            },
        );
        config.save(&path).unwrap();

        let error = remove(&path, &store, "main").unwrap_err();
        assert!(error.to_string().contains("acme"), "got: {error}");

        let mut config = Config::load(&path).unwrap();
        config.accounts.clear();
        config.save(&path).unwrap();
        remove(&path, &store, "main").unwrap();
        assert!(store.get_app("main").unwrap().is_none());
        assert_eq!(Config::load(&path).unwrap().default_app, None);
    }
}
