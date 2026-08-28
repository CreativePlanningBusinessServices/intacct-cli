use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CliError;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub default_account: Option<String>,
    #[serde(default)]
    pub accounts: BTreeMap<String, AccountEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_app: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub apps: BTreeMap<String, AppEntry>,
    #[serde(default)]
    pub cache_ttl_hours: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountEntry {
    pub company_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    pub flow: AuthFlow,
}

/// A registered Sage client application the CLI can reference by name. Only the
/// non-secret half lives in the config file; the secret is in the OS keychain.
#[derive(Debug, Serialize, Deserialize)]
pub struct AppEntry {
    pub client_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum AuthFlow {
    ClientCredentials,
    AuthCode,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, CliError> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let raw = std::fs::read_to_string(path).map_err(|read_error| {
            CliError::Usage(format!(
                "cannot read config {}: {read_error}",
                path.display()
            ))
        })?;
        toml::from_str(&raw).map_err(|parse_error| {
            CliError::Usage(format!("invalid config {}: {parse_error}", path.display()))
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), CliError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|io_error| {
                CliError::Usage(format!("cannot create {}: {io_error}", parent.display()))
            })?;
        }
        let raw = toml::to_string_pretty(self).expect("config serializable");
        std::fs::write(path, raw).map_err(|io_error| {
            CliError::Usage(format!("cannot write {}: {io_error}", path.display()))
        })
    }

    pub fn resolve_alias(&self, flag: Option<&str>, env: Option<&str>) -> Result<String, CliError> {
        let alias = flag
            .map(str::to_string)
            .or_else(|| env.map(str::to_string))
            .or_else(|| self.default_account.clone())
            .ok_or_else(|| CliError::Usage(
                "no account selected: pass --account, set INTACCT_ACCOUNT, or run `intacct-cli account set-default`".into()))?;
        if !self.accounts.contains_key(&alias) {
            return Err(CliError::Usage(format!(
                "unknown account alias '{alias}'; run `intacct-cli account list`"
            )));
        }
        Ok(alias)
    }

    pub fn resolve_app_name(&self, flag: Option<&str>) -> Result<String, CliError> {
        let name = flag
            .map(str::to_string)
            .or_else(|| self.default_app.clone())
            .ok_or_else(|| CliError::Usage(
                "no client app selected: pass --app, run `intacct-cli app add`, or provide --client-id".into()))?;
        if !self.apps.contains_key(&name) {
            return Err(CliError::Usage(format!(
                "unknown client app '{name}'; run `intacct-cli app list`"
            )));
        }
        Ok(name)
    }
}

pub fn default_config_path() -> PathBuf {
    directories::ProjectDirs::from("com", "CreativePlanning", "intacct-cli")
        .expect("resolvable home directory")
        .config_dir()
        .join("config.toml")
}

pub fn default_cache_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "CreativePlanning", "intacct-cli")
        .expect("resolvable home directory")
        .cache_dir()
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> Config {
        let mut config = Config::default();
        config.accounts.insert(
            "prod".into(),
            AccountEntry {
                company_id: "creativeplanning".into(),
                user_id: Some("svc_api".into()),
                entity_id: None,
                app: None,
                flow: AuthFlow::ClientCredentials,
            },
        );
        config.accounts.insert(
            "sandbox".into(),
            AccountEntry {
                company_id: "creativeplanning-snd".into(),
                user_id: None,
                entity_id: Some("CentralUS-35".into()),
                app: Some("main".into()),
                flow: AuthFlow::AuthCode,
            },
        );
        config.default_account = Some("prod".into());
        config.apps.insert(
            "main".into(),
            AppEntry {
                client_id: "cid.app.sage.com".into(),
            },
        );
        config.default_app = Some("main".into());
        config
    }

    #[test]
    fn config_round_trips_through_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        sample_config().save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.default_account.as_deref(), Some("prod"));
        assert_eq!(loaded.accounts["prod"].user_id.as_deref(), Some("svc_api"));
        assert_eq!(
            loaded.accounts["sandbox"].entity_id.as_deref(),
            Some("CentralUS-35")
        );
        assert!(matches!(
            loaded.accounts["sandbox"].flow,
            AuthFlow::AuthCode
        ));
    }

    #[test]
    fn missing_config_file_loads_as_default() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = Config::load(&dir.path().join("nope.toml")).unwrap();
        assert!(loaded.accounts.is_empty());
    }

    #[test]
    fn apps_round_trip_through_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        sample_config().save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.default_app.as_deref(), Some("main"));
        assert_eq!(loaded.apps["main"].client_id, "cid.app.sage.com");
        assert_eq!(loaded.accounts["sandbox"].app.as_deref(), Some("main"));
        assert_eq!(loaded.accounts["prod"].app, None);
    }

    #[test]
    fn app_name_resolution_prefers_flag_then_default() {
        let config = sample_config();
        assert_eq!(config.resolve_app_name(Some("main")).unwrap(), "main");
        assert_eq!(config.resolve_app_name(None).unwrap(), "main");
        assert!(matches!(
            config.resolve_app_name(Some("nope")),
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            Config::default().resolve_app_name(None),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn alias_resolution_prefers_flag_then_env_then_default() {
        let config = sample_config();
        assert_eq!(
            config
                .resolve_alias(Some("sandbox"), Some("ignored"))
                .unwrap(),
            "sandbox"
        );
        assert_eq!(
            config.resolve_alias(None, Some("sandbox")).unwrap(),
            "sandbox"
        );
        assert_eq!(config.resolve_alias(None, None).unwrap(), "prod");
        assert!(matches!(
            config.resolve_alias(Some("nope"), None),
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            Config::default().resolve_alias(None, None),
            Err(CliError::Usage(_))
        ));
    }
}
