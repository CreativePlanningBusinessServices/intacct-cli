mod common;

use intacct_cli::commands::view;
use serde_json::json;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn run_posts_key_and_view_type() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/services/core/view"))
        .and(body_json(json!({"key": "expenses/employee-expense::systemfw1", "viewType": "system", "size": 10})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::result": [{"key": "1"}], "ia::meta": {"totalCount": 1, "next": null}
        })))
        .mount(&server).await;
    let result = view::run(
        &common::client_for(&server),
        "expenses/employee-expense::systemfw1",
        "system",
        None,
        Some(10),
    )
    .await
    .unwrap();
    assert_eq!(result["ia::meta"]["totalCount"], 1);
}

#[tokio::test]
async fn list_system_views_passes_object_name() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/objects/core/system-view"))
        .and(query_param("name", "accounts-payable/vendor"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ia::result": []})))
        .expect(1)
        .mount(&server)
        .await;
    view::list_system_views(&common::client_for(&server), "accounts-payable/vendor")
        .await
        .unwrap();
}
