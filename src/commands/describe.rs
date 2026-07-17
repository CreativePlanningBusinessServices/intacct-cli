use std::path::Path;
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::client::IaClient;
use crate::error::CliError;

pub async fn describe_resource(
    client: &IaClient,
    name: &str,
    full_schema: bool,
    cache_dir: &Path,
    refresh: bool,
    cache_ttl: Duration,
) -> Result<Value, CliError> {
    let suffix = if full_schema { "full" } else { "short" };
    let cache_file = cache_dir.join(format!("{}.{suffix}.json", sanitize(name)));
    if !refresh && let Some(cached_metadata) = read_fresh_cache(&cache_file, cache_ttl) {
        return Ok(cached_metadata);
    }
    let response = client
        .request(
            reqwest::Method::GET,
            "/services/core/model",
            &[
                ("name", name.to_string()),
                ("schema", full_schema.to_string()),
            ],
            &[],
            None,
        )
        .await?;
    let metadata = response.body.unwrap_or(Value::Null);
    let _ = std::fs::create_dir_all(cache_dir);
    let _ = std::fs::write(
        &cache_file,
        serde_json::to_vec(&metadata).expect("metadata is serializable"),
    );
    Ok(metadata)
}

pub async fn list_resources(
    client: &IaClient,
    resource_type: &str,
    name_filter: Option<String>,
) -> Result<Value, CliError> {
    let mut query = vec![("type", resource_type.to_string())];
    if let Some(filter) = name_filter {
        query.push(("filter", filter));
    }
    let response = client
        .request(
            reqwest::Method::GET,
            "/services/core/model",
            &query,
            &[],
            None,
        )
        .await?;
    Ok(response.body.unwrap_or(Value::Null))
}

fn sanitize(name: &str) -> String {
    name.replace('/', "_").replace("::", "_")
}

fn read_fresh_cache(cache_file: &Path, cache_ttl: Duration) -> Option<Value> {
    let modified_at = std::fs::metadata(cache_file).ok()?.modified().ok()?;
    if SystemTime::now().duration_since(modified_at).ok()? > cache_ttl {
        return None;
    }
    serde_json::from_slice(&std::fs::read(cache_file).ok()?).ok()
}
