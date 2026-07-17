mod common;

use intacct_cli::commands::report;
use serde_json::json;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn run_submits_stored_report() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/services/reports/stored-reports"))
        .and(body_json(
            json!({"reportId": "1", "outputType": "pdf", "outputLocation": "intacct"}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ia::result": {"status": "submitted"}})),
        )
        .mount(&server)
        .await;
    let result = report::run(&common::client_for(&server), "1", "pdf", "intacct")
        .await
        .unwrap();
    assert_eq!(result["ia::result"]["status"], "submitted");
}

#[tokio::test]
async fn status_sends_all_three_query_params() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/reports/status"))
        .and(query_param("reportId", "1"))
        .and(query_param("outputType", "pdf"))
        .and(query_param("outputLocation", "intacct"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ia::result": {"status": "pending"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let result = report::status(&common::client_for(&server), "1", "pdf", "intacct")
        .await
        .unwrap();
    assert_eq!(result["ia::result"]["status"], "pending");
}

#[tokio::test]
async fn download_writes_binary() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/reports/download"))
        .and(query_param("reportId", "1"))
        .and(query_param("outputType", "pdf"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/pdf")
                .set_body_bytes(b"%PDF-1.4 fake".to_vec()),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("report.pdf");
    let result = report::download(&common::client_for(&server), "1", "pdf", &output)
        .await
        .unwrap();
    assert_eq!(result["bytes"], 13);
    assert_eq!(std::fs::read(&output).unwrap(), b"%PDF-1.4 fake".to_vec());
}

#[tokio::test]
async fn cancel_posts_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/services/reports/cancel"))
        .and(body_json(
            json!({"reportId": "1", "outputType": "pdf", "outputLocation": "intacct"}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ia::result": {"status": "canceled"}})),
        )
        .mount(&server)
        .await;
    let result = report::cancel(&common::client_for(&server), "1", "pdf", "intacct")
        .await
        .unwrap();
    assert_eq!(result["ia::result"]["status"], "canceled");
}
