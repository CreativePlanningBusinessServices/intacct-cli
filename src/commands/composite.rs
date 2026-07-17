use serde_json::Value;

use crate::client::IaClient;
use crate::error::CliError;

pub async fn run(client: &IaClient, sub_requests: Value) -> Result<Value, CliError> {
    // Validate that sub_requests is a JSON array
    if !sub_requests.is_array() {
        return Err(CliError::Usage(
            "composite takes a JSON array of 2-10 sub-requests".into(),
        ));
    }

    // Validate the array length (2-10 elements)
    let arr = sub_requests.as_array().expect("checked is_array above");
    if arr.len() < 2 || arr.len() > 10 {
        return Err(CliError::Usage(
            "composite takes a JSON array of 2-10 sub-requests".into(),
        ));
    }

    // POST the array verbatim to /services/core/composite
    let response = client
        .request(
            reqwest::Method::POST,
            "/services/core/composite",
            &[],
            &[],
            Some(&sub_requests),
        )
        .await?;

    Ok(response.body.unwrap_or(Value::Null))
}

pub async fn session_id(client: &IaClient) -> Result<Value, CliError> {
    // GET /services/core/session/id (two path segments, not session-id)
    let response = client
        .request(
            reqwest::Method::GET,
            "/services/core/session/id",
            &[],
            &[],
            None,
        )
        .await?;

    Ok(response.body.unwrap_or(Value::Null))
}
