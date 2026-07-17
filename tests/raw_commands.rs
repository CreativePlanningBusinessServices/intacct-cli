mod common;

use intacct_cli::commands::raw;
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn get_with_query_and_custom_header_reaches_mock() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/core/model"))
        .and(query_param("type", "service"))
        .and(header("X-Custom-Header", "test-value"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "metadata": {"type": "service", "name": "core"}
        })))
        .mount(&server)
        .await;

    let result = raw::call(
        &common::client_for(&server),
        reqwest::Method::GET,
        "/services/core/model",
        &[("type".to_string(), "service".to_string())],
        &[("X-Custom-Header".to_string(), "test-value".to_string())],
        None,
    )
    .await
    .unwrap();

    assert_eq!(result["metadata"]["type"], "service");
}

#[tokio::test]
async fn empty_response_body_yields_status_json() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/objects/accounts-payable/vendor/42"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let result = raw::call(
        &common::client_for(&server),
        reqwest::Method::DELETE,
        "/objects/accounts-payable/vendor/42",
        &[],
        &[],
        None,
    )
    .await
    .unwrap();

    assert_eq!(result["status"], 204);
}
