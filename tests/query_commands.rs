mod common;

use intacct_cli::commands::query;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn run_without_all_passes_through_single_page() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/services/core/query"))
        .and(body_partial_json(
            json!({"object": "x/y", "fields": ["key"]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::result": [{"key": "1"}],
            "ia::meta": {"totalCount": 1, "next": null}
        })))
        .mount(&server)
        .await;
    let body = json!({"object": "x/y", "fields": ["key"]});
    let result = query::run(&common::client_for(&server), body, false)
        .await
        .unwrap();
    assert_eq!(result["ia::result"], json!([{"key": "1"}]));
    assert_eq!(result["ia::meta"]["next"], json!(null));
}

#[tokio::test]
async fn run_all_merges_pages_following_next() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/services/core/query"))
        .and(body_partial_json(json!({"object": "x/y", "start": 1})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::result": [{"key": "1"}, {"key": "2"}],
            "ia::meta": {"totalCount": 3, "next": 3}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/services/core/query"))
        .and(body_partial_json(json!({"object": "x/y", "start": 3})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::result": [{"key": "3"}],
            "ia::meta": {"totalCount": 3, "next": null}
        })))
        .mount(&server)
        .await;
    let body = json!({"object": "x/y", "fields": ["key"], "start": 1});
    let result = query::run(&common::client_for(&server), body, true)
        .await
        .unwrap();
    assert_eq!(result["count"], 3);
    assert_eq!(result["hasMore"], false);
    assert_eq!(result["totalCount"], 3);
    assert_eq!(result["items"].as_array().unwrap().len(), 3);
}
