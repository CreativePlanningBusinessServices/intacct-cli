use serde_json::{Value, json};

use crate::client::IaClient;
use crate::error::CliError;

pub async fn submit(
    client: &IaClient,
    object_name: &str,
    operation: &str,
    data: Value,
    callback_url: Option<String>,
) -> Result<Value, CliError> {
    if !data.is_array() {
        return Err(CliError::Usage(
            "bulk jobs take a JSON array of objects".into(),
        ));
    }

    let mut request_body_json = json!({
        "objectName": object_name,
        "operation": operation,
        "jobFile": "file",
        "fileContentType": "json",
    });
    if let Some(url) = callback_url {
        request_body_json["callbackURL"] = json!(url);
    }

    let request_body = serde_json::to_string(&request_body_json).expect("serializable");
    let data_bytes = serde_json::to_vec(&data).expect("serializable");
    let build_form = move || {
        reqwest::multipart::Form::new()
            .part(
                "ia::requestBody",
                reqwest::multipart::Part::text(request_body.clone())
                    .mime_str("application/json")
                    .expect("static mime"),
            )
            .part(
                "file",
                reqwest::multipart::Part::bytes(data_bytes.clone())
                    .file_name("job.json")
                    .mime_str("application/json")
                    .expect("static mime"),
            )
    };

    let response = client
        .request_multipart("/services/bulk/job/create", &build_form)
        .await?;
    Ok(response.body.unwrap_or(Value::Null))
}

pub async fn status(client: &IaClient, job_id: &str, download: bool) -> Result<Value, CliError> {
    let response = client
        .request(
            reqwest::Method::GET,
            "/services/bulk/job/status",
            &[
                ("jobId", job_id.to_string()),
                ("download", download.to_string()),
            ],
            &[],
            None,
        )
        .await?;
    Ok(response.body.unwrap_or(Value::Null))
}
