use std::path::Path;

use serde_json::{Value, json};

use crate::client::IaClient;
use crate::commands::{refuse_existing_output, write_binary_output};
use crate::error::CliError;

pub async fn run(
    client: &IaClient,
    file_type: &str,
    query_body: Value,
    output_path: &Path,
) -> Result<Value, CliError> {
    refuse_existing_output(output_path)?;

    let body = json!({
        "fileType": file_type,
        "query": query_body,
    });
    let response = client
        .request_binary(
            reqwest::Method::POST,
            "/services/core/export",
            &[],
            Some(&body),
        )
        .await?;

    write_binary_output(output_path, response)
}
