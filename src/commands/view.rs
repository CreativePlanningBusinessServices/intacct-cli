use serde_json::{Value, json};

use crate::client::IaClient;
use crate::error::CliError;

pub async fn run(
    client: &IaClient,
    view_key: &str,
    view_type: &str,
    start: Option<u64>,
    size: Option<u64>,
) -> Result<Value, CliError> {
    let mut body = json!({
        "key": view_key,
        "viewType": view_type,
    });
    if let Some(start) = start {
        body["start"] = json!(start);
    }
    if let Some(size) = size {
        body["size"] = json!(size);
    }

    let response = client
        .request(
            reqwest::Method::POST,
            "/services/core/view",
            &[],
            &[],
            Some(&body),
        )
        .await?;
    Ok(response.body.unwrap_or(Value::Null))
}

pub async fn list_system_views(client: &IaClient, object_name: &str) -> Result<Value, CliError> {
    let response = client
        .request(
            reqwest::Method::GET,
            "/objects/core/system-view",
            &[("name", object_name.to_string())],
            &[],
            None,
        )
        .await?;
    Ok(response.body.unwrap_or(Value::Null))
}
