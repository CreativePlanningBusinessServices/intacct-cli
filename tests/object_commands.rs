mod common;

use intacct_cli::commands::object;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn get_fetches_by_path_and_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/objects/accounts-payable/vendor/42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::result": {"key": "42", "id": "V-42", "href": "/objects/accounts-payable/vendor/42"}
        })))
        .mount(&server)
        .await;
    let result = object::get(
        &common::client_for(&server),
        "accounts-payable/vendor",
        "42",
    )
    .await
    .unwrap();
    assert_eq!(result["ia::result"]["id"], "V-42");
}

#[tokio::test]
async fn create_sends_atomic_and_idempotency_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/objects/accounts-payable/vendor"))
        .and(header("X-IA-API-Param-Transaction", "true"))
        .and(header("Idempotency-Key", "abc-123"))
        .and(body_json(json!([{"id": "V1"}, {"id": "V2"}])))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "ia::result": [{"key": "1"}, {"key": "2"}],
            "ia::meta": {"totalCount": 2, "totalSuccess": 2, "totalError": 0}
        })))
        .mount(&server)
        .await;
    let result = object::create(
        &common::client_for(&server),
        "accounts-payable/vendor",
        json!([{"id": "V1"}, {"id": "V2"}]),
        true,
        Some("abc-123".into()),
    )
    .await
    .unwrap();
    assert_eq!(result["ia::meta"]["totalSuccess"], 2);
}

#[tokio::test]
async fn batch_update_without_array_data_is_a_usage_error() {
    let server = MockServer::start().await;
    let error = object::update(
        &common::client_for(&server),
        "accounts-payable/vendor",
        None,
        json!({"id": "V1"}),
        false,
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, intacct_cli::error::CliError::Usage(_)));
}

#[tokio::test]
async fn delete_returns_synthesized_acknowledgment() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/objects/accounts-payable/vendor/42,43"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let result = object::delete(
        &common::client_for(&server),
        "accounts-payable/vendor",
        "42,43",
    )
    .await
    .unwrap();
    assert_eq!(result, json!({"deleted": true, "keys": ["42", "43"]}));
}

#[tokio::test]
async fn list_all_merges_pages_following_next() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/objects/general-ledger/account"))
        .and(query_param("start", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::result": [{"key": "1"}, {"key": "2"}],
            "ia::meta": {"totalCount": 3, "start": 1, "pageSize": 2, "next": 3, "previous": null}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/objects/general-ledger/account"))
        .and(query_param("start", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::result": [{"key": "3"}],
            "ia::meta": {"totalCount": 3, "start": 3, "pageSize": 2, "next": null, "previous": 1}
        })))
        .mount(&server)
        .await;
    let result = object::list(
        &common::client_for(&server),
        "general-ledger/account",
        Some(1),
        Some(2),
        true,
    )
    .await
    .unwrap();
    assert_eq!(result["count"], 3);
    assert_eq!(result["hasMore"], false);
    assert_eq!(result["totalCount"], 3);
    assert_eq!(result["items"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn list_all_follows_string_next_url_without_reappending_params() {
    let server = MockServer::start().await;
    let next_url = format!(
        "{}/objects/general-ledger/account?start=3&size=2",
        server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/objects/general-ledger/account"))
        .and(query_param("size", "2"))
        .and(query_param_is_missing("start"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::result": [{"key": "1"}, {"key": "2"}],
            "ia::meta": {"totalCount": 3, "start": 1, "pageSize": 2, "next": next_url, "previous": null}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/objects/general-ledger/account"))
        .and(query_param("start", "3"))
        .and(query_param("size", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::result": [{"key": "3"}],
            "ia::meta": {"totalCount": 3, "start": 3, "pageSize": 2, "next": null, "previous": 1}
        })))
        .mount(&server)
        .await;
    let result = object::list(
        &common::client_for(&server),
        "general-ledger/account",
        None,
        Some(2),
        true,
    )
    .await
    .unwrap();
    assert_eq!(result["count"], 3);
    assert_eq!(result["hasMore"], false);
    assert_eq!(result["items"].as_array().unwrap().len(), 3);

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 2);
    let second_request_query = received[1].url.query().unwrap_or("");
    assert_eq!(second_request_query, "start=3&size=2");
}
