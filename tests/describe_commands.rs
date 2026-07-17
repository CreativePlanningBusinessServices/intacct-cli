mod common;

use std::time::Duration;

use intacct_cli::commands::describe;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn describe_fetches_and_caches() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/core/model"))
        .and(query_param("name", "accounts-payable/vendor"))
        .and(query_param("schema", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "vendor"})))
        .expect(1)
        .mount(&server)
        .await;
    let cache_dir = tempfile::tempdir().unwrap();
    let client = common::client_for(&server);
    let cache_ttl = Duration::from_secs(24 * 60 * 60);

    let first = describe::describe_resource(
        &client,
        "accounts-payable/vendor",
        true,
        cache_dir.path(),
        false,
        cache_ttl,
    )
    .await
    .unwrap();
    assert_eq!(first["name"], "vendor");

    let second = describe::describe_resource(
        &client,
        "accounts-payable/vendor",
        true,
        cache_dir.path(),
        false,
        cache_ttl,
    )
    .await
    .unwrap();
    assert_eq!(second["name"], "vendor");
}

#[tokio::test]
async fn refresh_bypasses_cache() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/core/model"))
        .and(query_param("name", "accounts-payable/vendor"))
        .and(query_param("schema", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "vendor"})))
        .expect(2)
        .mount(&server)
        .await;
    let cache_dir = tempfile::tempdir().unwrap();
    let client = common::client_for(&server);
    let cache_ttl = Duration::from_secs(24 * 60 * 60);

    describe::describe_resource(
        &client,
        "accounts-payable/vendor",
        true,
        cache_dir.path(),
        false,
        cache_ttl,
    )
    .await
    .unwrap();

    describe::describe_resource(
        &client,
        "accounts-payable/vendor",
        true,
        cache_dir.path(),
        true,
        cache_ttl,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn list_passes_type_and_filter() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/core/model"))
        .and(query_param("type", "object"))
        .and(query_param("filter", "^accounts-payable/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": []})))
        .expect(1)
        .mount(&server)
        .await;
    let client = common::client_for(&server);

    let result =
        describe::list_resources(&client, "object", Some("^accounts-payable/".to_string()))
            .await
            .unwrap();
    assert_eq!(result["items"], json!([]));
}
