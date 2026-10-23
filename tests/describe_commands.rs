mod common;

use std::time::Duration;

use intacct_cli::commands::describe;
use intacct_cli::error::CliError;
use serde_json::json;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
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

// The model service only filters on exact types (rootObject, coreService, ...), never on the
// object/service families the CLI exposes, so listing fetches everything and filters locally.
async fn mount_full_model_catalog(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/services/core/model"))
        .and(query_param_is_missing("type"))
        .and(query_param_is_missing("filter"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::meta": {"totalCount": 6, "totalError": 0, "totalSuccess": 6},
            "ia::result": [
                {"apiObject": "objects/accounts-payable/vendor", "type": "rootObject"},
                {"apiObject": "objects/accounts-payable/bill-line", "type": "ownedObject"},
                {"apiObject": "objects/accounts-receivable/customer", "type": "rootObject"},
                {"apiObject": "services/core/composite", "type": "coreService"},
                {"apiObject": "services/cash-management/bank-file/generate", "type": "functionService"},
                {"apiObject": "workflows/accounts-payable/vendor/approve", "type": "workflow"},
            ],
        })))
        .expect(1)
        .mount(server)
        .await;
}

fn listed_names(result: &serde_json::Value) -> Vec<&str> {
    result["ia::result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|resource| resource["apiObject"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn list_object_keeps_every_object_subtype() {
    let server = MockServer::start().await;
    mount_full_model_catalog(&server).await;
    let client = common::client_for(&server);

    let result = describe::list_resources(&client, "object", None)
        .await
        .unwrap();

    assert_eq!(
        listed_names(&result),
        [
            "objects/accounts-payable/vendor",
            "objects/accounts-payable/bill-line",
            "objects/accounts-receivable/customer",
        ]
    );
    assert_eq!(result["ia::meta"]["totalCount"], 3);
    assert_eq!(result["ia::meta"]["totalSuccess"], 3);
}

#[tokio::test]
async fn list_service_keeps_every_service_subtype() {
    let server = MockServer::start().await;
    mount_full_model_catalog(&server).await;
    let client = common::client_for(&server);

    let result = describe::list_resources(&client, "service", None)
        .await
        .unwrap();

    assert_eq!(
        listed_names(&result),
        [
            "services/core/composite",
            "services/cash-management/bank-file/generate",
        ]
    );
}

#[tokio::test]
async fn list_filter_is_a_regex_over_the_describe_name() {
    let server = MockServer::start().await;
    mount_full_model_catalog(&server).await;
    let client = common::client_for(&server);

    let result = describe::list_resources(
        &client,
        "object",
        Some("^accounts-payable/(vendor|bill-line)$".to_string()),
    )
    .await
    .unwrap();

    assert_eq!(
        listed_names(&result),
        [
            "objects/accounts-payable/vendor",
            "objects/accounts-payable/bill-line",
        ]
    );
}

#[tokio::test]
async fn list_rejects_an_invalid_filter_regex() {
    let server = MockServer::start().await;
    let client = common::client_for(&server);

    let error = describe::list_resources(&client, "workflow", Some("(".to_string()))
        .await
        .unwrap_err();

    assert!(matches!(error, CliError::Usage(_)), "got {error:?}");
}
