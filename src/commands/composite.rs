use serde_json::Value;

use crate::client::IaClient;
use crate::error::CliError;

pub async fn run(client: &IaClient, sub_requests: Value) -> Result<Value, CliError> {
    if !sub_requests.is_array() {
        return Err(CliError::Usage(
            "composite takes a JSON array of 2-10 sub-requests".into(),
        ));
    }

    let sub_request_list = sub_requests.as_array().expect("checked is_array above");
    if sub_request_list.len() < 2 || sub_request_list.len() > 10 {
        return Err(CliError::Usage(
            "composite takes a JSON array of 2-10 sub-requests".into(),
        ));
    }

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
