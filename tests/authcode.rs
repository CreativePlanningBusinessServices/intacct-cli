mod common;

use std::sync::Arc;

use intacct_cli::auth::TokenProvider;
use intacct_cli::auth::authcode::AuthCodeProvider;
use intacct_cli::secrets::{AccountSecrets, MemoryStore, SecretStore};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn provider_for(server: &MockServer, store: Arc<MemoryStore>) -> AuthCodeProvider {
    AuthCodeProvider::new(
        reqwest::Client::new(),
        "prod".into(),
        format!("{}/oauth2/token", server.uri()),
        store,
    )
}

fn seed_auth_code_secrets(store: &MemoryStore, refresh_token: Option<&str>) {
    store
        .set(
            "prod",
            &AccountSecrets::AuthCode {
                client_id: "cid.app.sage.com".into(),
                client_secret: "shhh".into(),
                refresh_token: refresh_token.map(str::to_string),
            },
        )
        .unwrap();
}

#[tokio::test]
async fn refresh_persists_rotated_token_before_returning() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=OLD"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "A", "refresh_token": "NEW", "expires_in": 28800
        })))
        .expect(1)
        .mount(&server)
        .await;

    let store = Arc::new(MemoryStore::default());
    seed_auth_code_secrets(&store, Some("OLD"));
    let provider = provider_for(&server, store.clone());

    assert_eq!(provider.access_token().await.unwrap(), "A");

    match store.get("prod").unwrap().expect("secrets stored") {
        AccountSecrets::AuthCode { refresh_token, .. } => {
            assert_eq!(refresh_token.as_deref(), Some("NEW"));
        }
        other => panic!("wrong variant: {other:?}"),
    }
    let cached = store.get_token("prod").unwrap().expect("token cached");
    assert_eq!(cached.access_token, "A");
}

#[tokio::test]
async fn missing_refresh_token_is_auth_error_mentioning_account_add() {
    let server = MockServer::start().await; // no mocks mounted: any request would 404
    let store = Arc::new(MemoryStore::default());
    seed_auth_code_secrets(&store, None);
    let provider = provider_for(&server, store);

    let error = provider.access_token().await.unwrap_err();
    match error {
        intacct_cli::error::CliError::Auth(message) => {
            assert!(message.contains("account add"), "got: {message}");
        }
        other => panic!("wrong error kind: {other:?}"),
    }
}

#[tokio::test]
async fn refresh_4xx_is_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad refresh token"))
        .mount(&server)
        .await;

    let store = Arc::new(MemoryStore::default());
    seed_auth_code_secrets(&store, Some("OLD"));
    let provider = provider_for(&server, store);

    let error = provider.access_token().await.unwrap_err();
    match error {
        intacct_cli::error::CliError::Auth(message) => {
            assert!(message.contains("refresh failed"), "got: {message}");
            assert!(message.contains("400"), "got: {message}");
            assert!(message.contains("bad refresh token"), "got: {message}");
            assert!(message.contains("account add"), "got: {message}");
        }
        other => panic!("wrong error kind: {other:?}"),
    }
}
