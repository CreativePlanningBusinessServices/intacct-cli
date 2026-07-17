mod common;

use std::sync::Arc;

use intacct_cli::commands::account::{self, AddArgs};
use intacct_cli::config::AuthFlow;
use intacct_cli::secrets::{AccountSecrets, MemoryStore, SecretStore};
use serde_json::json;

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
