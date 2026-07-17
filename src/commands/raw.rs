use serde_json::{Value, json};

use crate::client::IaClient;
use crate::error::CliError;

pub async fn call(
    client: &IaClient,
    method: reqwest::Method,
    path: &str,
    query: &[(String, String)],
    headers: &[(String, String)],
    body: Option<Value>,
) -> Result<Value, CliError> {
    let query_pairs: Vec<(&str, String)> = query
        .iter()
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect();
    let header_pairs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let response = client
        .request(method, path, &query_pairs, &header_pairs, body.as_ref())
        .await?;

    // Non-JSON (or empty) success bodies come back as `body: None`; surface the status
    // rather than erroring.
    match response.body {
        Some(json_body) => Ok(json_body),
        None => Ok(json!({"status": response.status})),
    }
}
