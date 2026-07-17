mod common;

use std::sync::Arc;

use intacct_cli::auth::TokenProvider;
use intacct_cli::auth::client_credentials::{ClientCredentialsConfig, ClientCredentialsProvider};
use intacct_cli::secrets::{CachedToken, MemoryStore, SecretStore};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn provider_for(server: &MockServer, store: Arc<MemoryStore>) -> ClientCredentialsProvider {
    ClientCredentialsProvider::new(
        reqwest::Client::new(),
        "prod".into(),
        ClientCredentialsConfig {
            token_url: format!("{}/oauth2/token", server.uri()),
            client_id: "cid.app.sage.com".into(),
            client_secret: "shhh".into(),
            username: "svc_api@creativeplanning".into(),
        },
        store,
    )
}

#[tokio::test]
async fn fetches_and_caches_a_fresh_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_partial_json(json!({
            "grant_type": "client_credentials",
            "client_id": "cid.app.sage.com",
            "client_secret": "shhh",
            "username": "svc_api@creativeplanning",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "token_type": "Bearer", "access_token": "FRESH", "refresh_token": "rt", "expires_in": 43200
        })))
        .expect(1)
        .mount(&server)
        .await;

    let store = Arc::new(MemoryStore::default());
    let provider = provider_for(&server, store.clone());
    assert_eq!(provider.access_token().await.unwrap(), "FRESH");
    let cached = store.get_token("prod").unwrap().expect("token cached");
    assert_eq!(cached.access_token, "FRESH");
}

#[tokio::test]
async fn returns_cached_token_without_hitting_the_network() {
    let server = MockServer::start().await; // no mocks mounted: any request would 404
    let store = Arc::new(MemoryStore::default());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    store
        .set_token(
            "prod",
            &CachedToken {
                access_token: "CACHED".into(),
                expires_at_epoch: now + 3600,
            },
        )
        .unwrap();

    let provider = provider_for(&server, store);
    assert_eq!(provider.access_token().await.unwrap(), "CACHED");
}

#[tokio::test]
async fn token_endpoint_failure_is_an_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad client"))
        .mount(&server)
        .await;
    let provider = provider_for(&server, Arc::new(MemoryStore::default()));
    assert!(matches!(
        provider.access_token().await,
        Err(intacct_cli::error::CliError::Auth(_))
    ));
}
