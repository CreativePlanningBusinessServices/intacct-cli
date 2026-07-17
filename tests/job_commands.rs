mod common;

use intacct_cli::commands::job;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn submit_posts_multipart_with_request_body_and_file_parts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/services/bulk/job/create"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({
            "ia::result": {"jobId": "88.JOB1"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let result = job::submit(
        &common::client_for(&server),
        "accounts-payable/vendor",
        "create",
        json!([{"id": "V1"}]),
        None,
    )
    .await
    .unwrap();
    assert_eq!(result["ia::result"]["jobId"], "88.JOB1");

    // Verify the multipart body carried both parts.
    let requests = server.received_requests().await.unwrap();
    let body = String::from_utf8_lossy(&requests[0].body).to_string();
    assert!(
        body.contains(r#"name="ia::requestBody""#),
        "missing ia::requestBody part: {body}"
    );
    assert!(
        body.contains(r#""objectName":"accounts-payable/vendor""#)
            || body.contains(r#""objectName" : "accounts-payable/vendor""#)
    );
    assert!(body.contains(r#"name="file""#), "missing file part: {body}");
    assert!(body.contains(r#"[{"id":"V1"}]"#));
    let content_type = requests[0]
        .headers
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        content_type.starts_with("multipart/form-data"),
        "got {content_type}"
    );
}

#[tokio::test]
async fn submit_rejects_non_array_data() {
    let server = MockServer::start().await;
    let error = job::submit(
        &common::client_for(&server),
        "accounts-payable/vendor",
        "create",
        json!({"id": "V1"}),
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, intacct_cli::error::CliError::Usage(_)));
}

#[tokio::test]
async fn status_passes_job_id_and_download_flag() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/bulk/job/status"))
        .and(query_param("jobId", "88.JOB1"))
        .and(query_param("download", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ia::result": {"status": "completed"}
        })))
        .mount(&server)
        .await;
    let result = job::status(&common::client_for(&server), "88.JOB1", true)
        .await
        .unwrap();
    assert_eq!(result["ia::result"]["status"], "completed");
}
