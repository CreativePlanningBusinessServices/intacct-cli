use serde_json::{Value, json};

use crate::client::IaClient;
use crate::error::CliError;

pub async fn get(client: &IaClient, object_path: &str, keys: &str) -> Result<Value, CliError> {
    let response = client
        .request(
            reqwest::Method::GET,
            &format!("/objects/{object_path}/{keys}"),
            &[],
            &[],
            None,
        )
        .await?;
    Ok(response.body.unwrap_or(Value::Null))
}

/// Intacct's documented maximum page count for a single listing traversal.
const MAX_PAGES: u32 = 1000;

pub async fn list(
    client: &IaClient,
    object_path: &str,
    start: Option<u64>,
    size: Option<u64>,
    fetch_all: bool,
) -> Result<Value, CliError> {
    let mut next_start = start;
    let mut merged_items: Vec<Value> = Vec::new();
    let mut pages_fetched: u32 = 0;
    let mut next_url: Option<String> = None;
    loop {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(start_row) = next_start {
            query.push(("start", start_row.to_string()));
        }
        if let Some(page_size) = size {
            query.push(("size", page_size.to_string()));
        }
        let path = next_url
            .take()
            .unwrap_or_else(|| format!("/objects/{object_path}"));
        let response = client
            .request(reqwest::Method::GET, &path, &query, &[], None)
            .await?;
        let page = response.body.unwrap_or(Value::Null);
        if !fetch_all {
            return Ok(page);
        }
        pages_fetched += 1;
        let page_items = page["ia::result"].as_array().cloned().unwrap_or_default();
        let page_item_count = page_items.len();
        merged_items.extend(page_items);
        let total = page["ia::meta"]["totalCount"].clone();

        match &page["ia::meta"]["next"] {
            Value::Null => {
                return Ok(json!({
                    "items": merged_items, "count": merged_items.len(),
                    "totalCount": total, "hasMore": false,
                }));
            }
            next_marker => {
                if page_item_count == 0 || pages_fetched >= MAX_PAGES {
                    return Ok(json!({
                        "items": merged_items, "count": merged_items.len(),
                        "totalCount": total, "hasMore": true,
                    }));
                }
                // `next` is a row number on query-style pages and may be a URL on
                // list pages; support both.
                if let Some(row) = next_marker.as_u64() {
                    next_start = Some(row);
                } else if let Some(url) = next_marker.as_str() {
                    next_url = Some(url.to_string());
                    next_start = None;
                } else {
                    return Err(CliError::Api {
                        status: 200,
                        message: format!("unrecognized ia::meta.next value: {next_marker}"),
                        details: vec![],
                        support_id: None,
                    });
                }
            }
        }
    }
}

pub async fn create(
    client: &IaClient,
    object_path: &str,
    data: Value,
    atomic: bool,
    idempotency_key: Option<String>,
) -> Result<Value, CliError> {
    let headers = write_headers(atomic, &idempotency_key);
    let response = client
        .request(
            reqwest::Method::POST,
            &format!("/objects/{object_path}"),
            &[],
            &headers,
            Some(&data),
        )
        .await?;
    Ok(response.body.unwrap_or(Value::Null))
}

pub async fn update(
    client: &IaClient,
    object_path: &str,
    key: Option<&str>,
    data: Value,
    atomic: bool,
    idempotency_key: Option<String>,
) -> Result<Value, CliError> {
    let path = match key {
        Some(key) => format!("/objects/{object_path}/{key}"),
        None => {
            if !data.is_array() {
                return Err(CliError::Usage(
                    "batch update requires --data to be a JSON array of objects each carrying its key"
                        .into(),
                ));
            }
            format!("/objects/{object_path}")
        }
    };
    let headers = write_headers(atomic, &idempotency_key);
    let response = client
        .request(reqwest::Method::PATCH, &path, &[], &headers, Some(&data))
        .await?;
    Ok(response.body.unwrap_or(Value::Null))
}

pub async fn delete(client: &IaClient, object_path: &str, keys: &str) -> Result<Value, CliError> {
    client
        .request(
            reqwest::Method::DELETE,
            &format!("/objects/{object_path}/{keys}"),
            &[],
            &[],
            None,
        )
        .await?;
    let keys: Vec<&str> = keys.split(',').collect();
    Ok(json!({"deleted": true, "keys": keys}))
}

fn write_headers(atomic: bool, idempotency_key: &Option<String>) -> Vec<(&'static str, &str)> {
    let mut headers = Vec::new();
    if atomic {
        headers.push(("X-IA-API-Param-Transaction", "true"));
    }
    if let Some(key) = idempotency_key {
        headers.push(("Idempotency-Key", key.as_str()));
    }
    headers
}
