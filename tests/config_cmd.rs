mod common;

use intacct_cli::commands::account::{self, AddArgs};
use intacct_cli::commands::config_cmd;
use intacct_cli::config::AuthFlow;
use intacct_cli::error::CliError;
use intacct_cli::secrets::MemoryStore;

fn add_args(alias: &str) -> AddArgs {
    AddArgs {
        alias: alias.into(),
        company_id: "creativeplanning".into(),
        flow: AuthFlow::ClientCredentials,
        client_id: "cid.app.sage.com".into(),
        client_secret: "shhh".into(),
        user_id: Some("svc_api".into()),
        entity_id: None,
        port: 8899,
        paste: false,
    }
}

#[test]
fn set_default_account_validates_alias_exists() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");

    // Setting default_account to unknown alias should fail
    let result = config_cmd::set(&config_path, "default_account", "unknown");
    assert!(matches!(result, Err(CliError::Usage(_))));
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("unknown"));
}

#[tokio::test]
async fn set_default_account_with_known_alias_persists_and_get_returns_it() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let store = MemoryStore::default();
    let http = reqwest::Client::new();

    // Add an account first
    account::add(&config_path, &store, &http, add_args("prod"))
        .await
        .unwrap();

    // Now set default_account to the known alias
    let result = config_cmd::set(&config_path, "default_account", "prod").unwrap();
    assert_eq!(result["default_account"], "prod");

    // Verify that get("default_account") returns the set value
    let get_result = config_cmd::get(&config_path, Some("default_account")).unwrap();
    assert_eq!(get_result["default_account"], "prod");
}

#[test]
fn set_cache_ttl_rejects_non_numeric() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");

    let result = config_cmd::set(&config_path, "cache_ttl_hours", "not_a_number");
    assert!(matches!(result, Err(CliError::Usage(_))));
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("cache_ttl_hours"));
    assert!(err_msg.contains("integer"));
}

#[test]
fn set_cache_ttl_accepts_numeric_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");

    let result = config_cmd::set(&config_path, "cache_ttl_hours", "48").unwrap();
    assert_eq!(result["cache_ttl_hours"], 48u64);

    // Verify that get("cache_ttl_hours") returns the set value
    let get_result = config_cmd::get(&config_path, Some("cache_ttl_hours")).unwrap();
    assert_eq!(get_result["cache_ttl_hours"], 48u64);
}

#[test]
fn get_without_key_returns_effective_config_with_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");

    // Get with no file should return defaults
    let result = config_cmd::get(&config_path, None).unwrap();
    assert_eq!(result["cache_ttl_hours"], 24);
    assert!(result["default_account"].is_null());

    // Set cache_ttl_hours and default_account
    config_cmd::set(&config_path, "cache_ttl_hours", "36").unwrap();

    // Get full config again
    let result = config_cmd::get(&config_path, None).unwrap();
    assert_eq!(result["cache_ttl_hours"], 36);
}

#[test]
fn get_unknown_key_returns_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");

    let result = config_cmd::get(&config_path, Some("unknown_key"));
    assert!(matches!(result, Err(CliError::Usage(_))));
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("unknown_key"));
    assert!(err_msg.contains("valid keys"));
}

#[test]
fn set_unknown_key_returns_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");

    let result = config_cmd::set(&config_path, "unknown_key", "value");
    assert!(matches!(result, Err(CliError::Usage(_))));
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("unknown_key"));
    assert!(err_msg.contains("valid keys"));
}
