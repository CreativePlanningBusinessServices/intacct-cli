mod common;

use std::sync::Arc;

use intacct_cli::commands::account::{self, AddArgs};
use intacct_cli::config::AuthFlow;
use intacct_cli::error::CliError;
use intacct_cli::secrets::{AccountSecrets, AppSecrets, CachedToken, MemoryStore, SecretStore};
use serde_json::json;

/// A `SecretStore` whose `set` always fails, used to prove that `account::add` writes the
/// config entry before it ever touches the keychain — so a keychain failure still leaves a
/// re-runnable config behind rather than an invisible half-added account.
struct FailingSecretStore {
    inner: MemoryStore,
}

impl FailingSecretStore {
    fn new() -> Self {
        Self {
            inner: MemoryStore::default(),
        }
    }
}

impl SecretStore for FailingSecretStore {
    fn get(&self, alias: &str) -> Result<Option<AccountSecrets>, CliError> {
        self.inner.get(alias)
    }
    fn set(&self, _alias: &str, _secrets: &AccountSecrets) -> Result<(), CliError> {
        Err(CliError::Auth("keychain unavailable: simulated".into()))
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

fn add_args(alias: &str) -> AddArgs {
    AddArgs {
        alias: alias.into(),
        company_id: "creativeplanning".into(),
        flow: AuthFlow::ClientCredentials,
        credentials: account::CredentialSource::Inline {
            client_id: "cid.app.sage.com".into(),
            client_secret: "shhh".into(),
        },
        user_id: Some("svc_api".into()),
        entity_id: None,
        port: 8899,
        paste: false,
    }
}

#[tokio::test]
async fn add_with_app_source_stores_a_reference_not_copied_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let store = MemoryStore::default();
    let http = reqwest::Client::new();

    let mut args = add_args("acme");
    args.credentials = account::CredentialSource::App {
        name: "main".into(),
        client_id: "cid.app.sage.com".into(),
        client_secret: "shhh".into(),
    };
    let result = account::add(&config_path, &store, &http, args)
        .await
        .unwrap();
    assert_eq!(result["app"], "main");

    match store.get("acme").unwrap().expect("secrets stored") {
        AccountSecrets::ClientCredentialsApp { app, username } => {
            assert_eq!(app, "main");
            assert_eq!(username, "svc_api@creativeplanning");
        }
        other => panic!("expected an app reference, got: {other:?}"),
    }
    let config = intacct_cli::config::Config::load(&config_path).unwrap();
    assert_eq!(config.accounts["acme"].app.as_deref(), Some("main"));
}

#[tokio::test]
async fn add_stores_config_and_secrets_and_first_account_becomes_default() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let store = MemoryStore::default();
    let http = reqwest::Client::new();

    let result = account::add(&config_path, &store, &http, add_args("prod"))
        .await
        .unwrap();
    assert_eq!(
        result,
        json!({"alias": "prod", "companyId": "creativeplanning",
                              "flow": "client-credentials", "default": true})
    );

    match store.get("prod").unwrap().expect("secrets stored") {
        AccountSecrets::ClientCredentials { username, .. } => {
            assert_eq!(username, "svc_api@creativeplanning")
        }
        other => panic!("wrong variant: {other:?}"),
    }
    let listed = account::list(&config_path).unwrap();
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["accounts"][0]["default"], true);
    // list output must never contain secret material
    assert!(!listed.to_string().contains("shhh"));
}

#[tokio::test]
async fn add_requires_user_id_for_client_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let http = reqwest::Client::new();
    let mut args = add_args("prod");
    args.user_id = None;
    assert!(matches!(
        account::add(&dir.path().join("config.toml"), &store, &http, args).await,
        Err(intacct_cli::error::CliError::Usage(_))
    ));
}

#[tokio::test]
async fn remove_sweeps_config_default_and_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let store = MemoryStore::default();
    let http = reqwest::Client::new();
    account::add(&config_path, &store, &http, add_args("prod"))
        .await
        .unwrap();

    account::remove(&config_path, &store, "prod").unwrap();
    assert!(store.get("prod").unwrap().is_none());
    let listed = account::list(&config_path).unwrap();
    assert_eq!(listed["count"], 0);
}

#[tokio::test]
async fn revoke_without_yes_in_non_interactive_context_is_usage_error() {
    // cargo test's stdin is never a tty, so the interactive-confirmation branch is
    // unreachable here and revoke must refuse before making any network call.
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let store = MemoryStore::default();
    let http = reqwest::Client::new();
    account::add(&config_path, &store, &http, add_args("prod"))
        .await
        .unwrap();

    let store: Arc<dyn SecretStore> = Arc::new(store);
    let result = account::revoke(&config_path, store, "prod", &http, false).await;
    assert!(matches!(
        result,
        Err(intacct_cli::error::CliError::Usage(_))
    ));
}

#[tokio::test]
async fn revoke_rejects_unknown_alias() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
    let http = reqwest::Client::new();
    let result = account::revoke(&config_path, store, "nope", &http, true).await;
    assert!(matches!(
        result,
        Err(intacct_cli::error::CliError::Usage(_))
    ));
}

#[tokio::test]
async fn add_with_failing_keychain_leaves_rerunnable_config() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let failing_store = FailingSecretStore::new();
    let http = reqwest::Client::new();

    let add_result = account::add(&config_path, &failing_store, &http, add_args("prod")).await;
    assert!(matches!(add_result, Err(CliError::Auth(_))));

    // The config-before-keyring ordering means the account entry is already on disk even
    // though the keychain write failed.
    let config = intacct_cli::config::Config::load(&config_path).unwrap();
    assert!(config.accounts.contains_key("prod"));
    assert_eq!(config.default_account.as_deref(), Some("prod"));

    // Re-running add with a working store against the same config path succeeds.
    let working_store = MemoryStore::default();
    let retry_result = account::add(&config_path, &working_store, &http, add_args("prod"))
        .await
        .unwrap();
    assert_eq!(retry_result["alias"], "prod");
    assert!(working_store.get("prod").unwrap().is_some());
}

#[tokio::test]
async fn test_command_hits_the_model_service() {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/core/model"))
        .and(query_param("name", "company-config/user"))
        .and(query_param("schema", "false"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ia::result": {"name": "user"}})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let context = intacct_cli::context::AccountContext {
        alias: "prod".into(),
        company_id: "creativeplanning".into(),
        client: common::client_for(&server),
    };
    let result = account::test(&context).await.unwrap();
    assert_eq!(result["ok"], true);
}
