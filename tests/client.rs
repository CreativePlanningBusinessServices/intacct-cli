mod common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use intacct_cli::auth::TokenProvider;
use intacct_cli::client::IaClient;
use intacct_cli::error::CliError;
use serde_json::json;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn success_passes_body_through_and_sends_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/objects/accounts-payable/vendor/42"))
        .and(header("authorization", "Bearer TEST_TOKEN"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ia::result": {"key": "42"}})),
        )
        .mount(&server)
        .await;
    let client = common::client_for(&server);
    let response = client
        .request(
            reqwest::Method::GET,
            "/objects/accounts-payable/vendor/42",
            &[],
            &[],
            None,
        )
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body.unwrap()["ia::result"]["key"], "42");
}

#[tokio::test]
async fn entity_header_is_sent_when_configured() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/objects/accounts-payable/vendor"))
        .and(header("X-IA-API-Param-Entity", "CentralUS-35"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ia::result": []})))
        .expect(1)
        .mount(&server)
        .await;
    let client = IaClient::new(
        reqwest::Client::new(),
        server.uri(),
        Arc::new(common::StaticToken),
        Some("CentralUS-35".into()),
    );
    client
        .request(
            reqwest::Method::GET,
            "/objects/accounts-payable/vendor",
            &[],
            &[],
            None,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn intacct_error_body_maps_to_api_error_with_support_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/objects/x/y/1"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "ia::error": {
                "code": "operationFailed",
                "message": "Operation create object HQemployee failed",
                "supportId": "sojLj~X2",
                "details": [{"code": "BL01001973", "message": "Employee Contact info cannot be empty"}]
            }
        })))
        .mount(&server)
        .await;
    let client = common::client_for(&server);
    let error = client
        .request(reqwest::Method::GET, "/objects/x/y/1", &[], &[], None)
        .await
        .unwrap_err();
    match error {
        CliError::Api {
            status,
            message,
            details,
            support_id,
        } => {
            assert_eq!(status, 422);
            assert_eq!(
                message,
                "operationFailed: Operation create object HQemployee failed"
            );
            assert_eq!(details.len(), 1);
            assert_eq!(support_id.as_deref(), Some("sojLj~X2"));
        }
        other => panic!("wrong error: {other:?}"),
    }
}

struct CountingTokens(AtomicU32);
impl TokenProvider for CountingTokens {
    fn access_token<'life>(
        &'life self,
    ) -> Pin<Box<dyn Future<Output = Result<String, CliError>> + Send + 'life>> {
        let calls = self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(if calls == 0 {
                "STALE".into()
            } else {
                "FRESH".into()
            })
        })
    }
    fn invalidate(&self) {}
}

#[tokio::test]
async fn unauthorized_response_invalidates_and_retries_once() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/objects/a/b"))
        .and(header("authorization", "Bearer STALE"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(
                json!({"ia::error": {"code": "GW-0031", "message": "invalid token"}}),
            ),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/objects/a/b"))
        .and(header("authorization", "Bearer FRESH"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ia::result": []})))
        .mount(&server)
        .await;
    let client = IaClient::new(
        reqwest::Client::new(),
        server.uri(),
        Arc::new(CountingTokens(AtomicU32::new(0))),
        None,
    );
    let response = client
        .request(reqwest::Method::GET, "/objects/a/b", &[], &[], None)
        .await
        .unwrap();
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn retries_429_honoring_retry_after_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/limited"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/limited"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ia::result": []})))
        .mount(&server)
        .await;

    let client = common::client_for(&server);
    let response = client
        .request(reqwest::Method::GET, "/limited", &[], &[], None)
        .await
        .unwrap();
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn no_entity_header_when_not_configured() {
    let server = MockServer::start().await;
    // Mounted with higher priority (lower number) so it wins if the entity
    // header is ever sent; the plain 200 mock only matches when it's absent.
    Mock::given(method("GET"))
        .and(path("/objects/no-entity"))
        .and(header_exists("X-IA-API-Param-Entity"))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/objects/no-entity"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ia::result": []})))
        .with_priority(2)
        .mount(&server)
        .await;

    let client = common::client_for(&server);
    let response = client
        .request(reqwest::Method::GET, "/objects/no-entity", &[], &[], None)
        .await
        .unwrap();
    assert_eq!(response.status, 200);
}
