use std::path::Path;

use serde_json::{Value, json};

use crate::client::IaClient;
use crate::commands::{refuse_existing_output, write_binary_output};
use crate::error::CliError;

pub async fn run(
    client: &IaClient,
    report_id: &str,
    output_type: &str,
    output_location: &str,
) -> Result<Value, CliError> {
    let body = json!({
        "reportId": report_id,
        "outputType": output_type,
        "outputLocation": output_location,
    });
    let response = client
        .request(
            reqwest::Method::POST,
            "/services/reports/stored-reports",
            &[],
            &[],
            Some(&body),
        )
        .await?;
    Ok(response.body.unwrap_or(Value::Null))
}

pub async fn status(
    client: &IaClient,
    report_id: &str,
    output_type: &str,
    output_location: &str,
) -> Result<Value, CliError> {
    let response = client
        .request(
            reqwest::Method::GET,
            "/services/reports/status",
            &[
                ("reportId", report_id.to_string()),
                ("outputType", output_type.to_string()),
                ("outputLocation", output_location.to_string()),
            ],
            &[],
            None,
        )
        .await?;
    Ok(response.body.unwrap_or(Value::Null))
}

pub async fn download(
    client: &IaClient,
    report_id: &str,
    output_type: &str,
    output_path: &Path,
) -> Result<Value, CliError> {
    refuse_existing_output(output_path)?;

    let response = client
        .request_binary(
            reqwest::Method::GET,
            "/services/reports/download",
            &[
                ("reportId", report_id.to_string()),
                ("outputType", output_type.to_string()),
            ],
            None,
        )
        .await?;

    write_binary_output(output_path, response)
}

pub async fn cancel(
    client: &IaClient,
    report_id: &str,
    output_type: &str,
    output_location: &str,
) -> Result<Value, CliError> {
    let body = json!({
        "reportId": report_id,
        "outputType": output_type,
        "outputLocation": output_location,
    });
    let response = client
        .request(
            reqwest::Method::POST,
            "/services/reports/cancel",
            &[],
            &[],
            Some(&body),
        )
        .await?;
    Ok(response.body.unwrap_or(Value::Null))
}
