mod common;

use intacct_cli::commands::export;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn export_writes_binary_response_to_file() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/services/core/export"))
        .and(body_partial_json(
            json!({"fileType": "csv", "query": {"object": "accounts-payable/vendor"}}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/csv")
                .set_body_bytes(b"id,name\nV1,Acme\n".to_vec()),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("vendors.csv");
    let query_body = json!({"object": "accounts-payable/vendor", "fields": ["id", "name"]});
    let result = export::run(&common::client_for(&server), "csv", query_body, &output)
        .await
        .unwrap();
    assert_eq!(result["bytes"], 16);
    assert_eq!(
        std::fs::read_to_string(&output).unwrap(),
        "id,name\nV1,Acme\n"
    );
}

#[tokio::test]
async fn export_refuses_to_overwrite() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("exists.csv");
    std::fs::write(&output, "old").unwrap();
    let error = export::run(
        &common::client_for(&server),
        "csv",
        json!({"object": "x/y", "fields": ["id"]}),
        &output,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, intacct_cli::error::CliError::Usage(_)));
}
