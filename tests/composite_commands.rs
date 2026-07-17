mod common;

use intacct_cli::commands::composite;
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn composite_posts_array_and_passes_through() {
    let server = MockServer::start().await;
    let request_array = json!([
        {"method": "GET", "path": "/objects/accounts-payable/vendor/1"},
        {"method": "GET", "path": "/objects/accounts-payable/vendor/2"}
    ]);

    Mock::given(method("POST"))
        .and(path("/services/core/composite"))
        .and(body_json(request_array.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::result": [
                {"key": "V1", "id": "vendor1"},
                {"key": "V2", "id": "vendor2"}
            ]
        })))
        .mount(&server)
        .await;

    let result = composite::run(&common::client_for(&server), request_array)
        .await
        .unwrap();

    assert_eq!(result["ia::result"][0]["id"], "vendor1");
    assert_eq!(result["ia::result"][1]["id"], "vendor2");
}

#[tokio::test]
async fn composite_rejects_too_few_elements() {
    let server = MockServer::start().await;
    let request_array = json!([
        {"method": "GET", "path": "/objects/accounts-payable/vendor/1"}
    ]);

    let error = composite::run(&common::client_for(&server), request_array)
        .await
        .unwrap_err();
    assert!(matches!(error, intacct_cli::error::CliError::Usage(_)));
}

#[tokio::test]
async fn composite_rejects_too_many_elements() {
    let server = MockServer::start().await;
    let request_array = json!([
        {"method": "GET", "path": "/objects/accounts-payable/vendor/1"},
        {"method": "GET", "path": "/objects/accounts-payable/vendor/2"},
        {"method": "GET", "path": "/objects/accounts-payable/vendor/3"},
        {"method": "GET", "path": "/objects/accounts-payable/vendor/4"},
        {"method": "GET", "path": "/objects/accounts-payable/vendor/5"},
        {"method": "GET", "path": "/objects/accounts-payable/vendor/6"},
        {"method": "GET", "path": "/objects/accounts-payable/vendor/7"},
        {"method": "GET", "path": "/objects/accounts-payable/vendor/8"},
        {"method": "GET", "path": "/objects/accounts-payable/vendor/9"},
        {"method": "GET", "path": "/objects/accounts-payable/vendor/10"},
        {"method": "GET", "path": "/objects/accounts-payable/vendor/11"}
    ]);

    let error = composite::run(&common::client_for(&server), request_array)
        .await
        .unwrap_err();
    assert!(matches!(error, intacct_cli::error::CliError::Usage(_)));
}

#[tokio::test]
async fn session_id_gets_the_service() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/core/session/id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::result": {
                "sessionId": "CkAU0b5A7K9V3Q1W5X7Z9B2D4F6G8H0J",
                "expiresIn": 3600
            }
        })))
        .mount(&server)
        .await;

    let result = composite::session_id(&common::client_for(&server))
        .await
        .unwrap();

    assert_eq!(
        result["ia::result"]["sessionId"],
        "CkAU0b5A7K9V3Q1W5X7Z9B2D4F6G8H0J"
    );
    assert_eq!(result["ia::result"]["expiresIn"], 3600);
}
