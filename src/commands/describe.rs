use std::path::Path;
use std::time::{Duration, SystemTime};

use regex::Regex;
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

/// The model service's `type` param only matches exact types (rootObject, ownedObject,
/// coreService, ...), so the object/service families are filtered here instead.
pub async fn list_resources(
    client: &IaClient,
    resource_type: &str,
    name_filter: Option<String>,
) -> Result<Value, CliError> {
    let name_pattern = name_filter
        .map(|filter| {
            Regex::new(&filter)
                .map_err(|error| CliError::Usage(format!("invalid --filter regex: {error}")))
        })
        .transpose()?;
    let response = client
        .request(reqwest::Method::GET, "/services/core/model", &[], &[], None)
        .await?;
    let mut catalog = response.body.unwrap_or(Value::Null);
    let Some(resources) = catalog["ia::result"].as_array_mut() else {
        return Ok(catalog);
    };
    resources.retain(|resource| {
        belongs_to_family(resource, resource_type)
            && name_pattern
                .as_ref()
                .is_none_or(|pattern| pattern.is_match(describe_name(resource)))
    });
    let listed_count = resources.len();
    if let Some(meta) = catalog["ia::meta"].as_object_mut() {
        meta.insert("totalCount".into(), listed_count.into());
        meta.insert("totalSuccess".into(), listed_count.into());
    }
    Ok(catalog)
}

fn belongs_to_family(resource: &Value, resource_type: &str) -> bool {
    let model_type = resource["type"].as_str().unwrap_or_default();
    match resource_type {
        "object" => model_type.ends_with("Object"),
        "service" => model_type.ends_with("Service"),
        _ => model_type == resource_type,
    }
}

/// The name `describe` accepts: apiObject without its objects/, services/ or workflows/ prefix.
fn describe_name(resource: &Value) -> &str {
    let api_object = resource["apiObject"].as_str().unwrap_or_default();
    api_object
        .split_once('/')
        .map_or(api_object, |(_, name)| name)
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
