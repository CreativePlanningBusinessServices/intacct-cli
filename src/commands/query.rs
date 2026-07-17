use serde_json::{Map, Value, json};

use crate::client::IaClient;
use crate::error::CliError;

pub struct QueryArgs {
    pub object: String,
    pub fields: Option<String>,
    pub filters: Vec<String>,
    pub filter_expression: Option<String>,
    pub order_by: Vec<String>,
    pub start: Option<u64>,
    pub size: Option<u64>,
    pub as_of_date: Option<String>,
    pub case_sensitive: bool,
    pub include_private: bool,
    pub include_hierarchy_fields: bool,
}

/// Assembles the documented `POST /services/core/query` body from structured flags. Pure and
/// unit-tested; `run` is the only caller that talks to the network.
pub fn build_query_body(args: &QueryArgs) -> Result<Value, CliError> {
    let fields = args.fields.as_deref().ok_or_else(|| {
        CliError::Usage(
            "--fields is required (comma-separated; aggregates like sum:amount allowed)".into(),
        )
    })?;
    let fields: Vec<&str> = fields.split(',').collect();

    let mut body = json!({
        "object": args.object,
        "fields": fields,
    });

    if !args.filters.is_empty() {
        let filters = parse_json_values(&args.filters, "--filter")?;
        body["filters"] = json!(filters);
    }
    if let Some(filter_expression) = &args.filter_expression {
        body["filterExpression"] = json!(filter_expression);
    }
    if !args.order_by.is_empty() {
        let order_by = parse_json_values(&args.order_by, "--order-by")?;
        body["orderBy"] = json!(order_by);
    }

    let filter_parameters = build_filter_parameters(args);
    if !filter_parameters.is_empty() {
        body["filterParameters"] = Value::Object(filter_parameters);
    }

    if let Some(start) = args.start {
        body["start"] = json!(start);
    }
    if let Some(size) = args.size {
        body["size"] = json!(size);
    }

    Ok(body)
}

fn parse_json_values(raw_values: &[String], flag_name: &str) -> Result<Vec<Value>, CliError> {
    raw_values
        .iter()
        .map(|raw_value| {
            serde_json::from_str(raw_value).map_err(|parse_error| {
                CliError::Usage(format!("{flag_name} is not valid JSON: {parse_error}"))
            })
        })
        .collect()
}

fn build_filter_parameters(args: &QueryArgs) -> Map<String, Value> {
    let mut filter_parameters = Map::new();
    if let Some(as_of_date) = &args.as_of_date {
        filter_parameters.insert("asOfDate".into(), json!(as_of_date));
    }
    if args.case_sensitive {
        filter_parameters.insert("caseSensitiveComparison".into(), json!(true));
    }
    if args.include_private {
        filter_parameters.insert("includePrivate".into(), json!(true));
    }
    if args.include_hierarchy_fields {
        filter_parameters.insert("includeHierarchyFields".into(), json!(true));
    }
    filter_parameters
}

/// Intacct's documented maximum page count for a single query traversal.
const MAX_PAGES: u32 = 1000;

pub async fn run(client: &IaClient, mut body: Value, fetch_all: bool) -> Result<Value, CliError> {
    let mut merged_items: Vec<Value> = Vec::new();
    let mut pages_fetched: u32 = 0;
    loop {
        let response = client
            .request(
                reqwest::Method::POST,
                "/services/core/query",
                &[],
                &[],
                Some(&body),
            )
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
                // `next` is always a numeric row offset for the query service (unlike
                // `object list`, which can also receive a follow-as-is URL).
                let Some(next_start) = next_marker.as_u64() else {
                    return Err(CliError::Api {
                        status: 200,
                        message: format!("unrecognized ia::meta.next value: {next_marker}"),
                        details: vec![],
                        support_id: None,
                    });
                };
                if page_item_count == 0 || pages_fetched >= MAX_PAGES {
                    return Ok(json!({
                        "items": merged_items, "count": merged_items.len(),
                        "totalCount": total, "hasMore": true,
                    }));
                }
                body["start"] = json!(next_start);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_args() -> QueryArgs {
        QueryArgs {
            object: "accounts-payable/vendor".into(),
            fields: Some("key,id,vendor.creditLimit".into()),
            filters: vec![],
            filter_expression: None,
            order_by: vec![],
            start: None,
            size: None,
            as_of_date: None,
            case_sensitive: false,
            include_private: false,
            include_hierarchy_fields: false,
        }
    }

    #[test]
    fn builds_minimal_body() {
        let body = build_query_body(&base_args()).unwrap();
        assert_eq!(
            body,
            json!({
                "object": "accounts-payable/vendor",
                "fields": ["key", "id", "vendor.creditLimit"],
            })
        );
    }

    #[test]
    fn builds_full_body_with_filters_ordering_and_parameters() {
        let mut args = base_args();
        args.filters = vec![
            r#"{"$eq":{"status":"active"}}"#.into(),
            r#"{"$contains":{"name":"Acme"}}"#.into(),
        ];
        args.filter_expression = Some("1 and 2".into());
        args.order_by = vec![r#"{"totalDue":"asc"}"#.into()];
        args.start = Some(1);
        args.size = Some(100);
        args.as_of_date = Some("2026-04-01".into());
        args.case_sensitive = true;
        let body = build_query_body(&args).unwrap();
        assert_eq!(
            body["filters"],
            json!([{"$eq":{"status":"active"}}, {"$contains":{"name":"Acme"}}])
        );
        assert_eq!(body["filterExpression"], "1 and 2");
        assert_eq!(body["orderBy"], json!([{"totalDue":"asc"}]));
        assert_eq!(
            body["filterParameters"],
            json!({"asOfDate":"2026-04-01","caseSensitiveComparison":true})
        );
        assert_eq!(body["start"], 1);
        assert_eq!(body["size"], 100);
    }

    #[test]
    fn invalid_filter_json_is_a_usage_error() {
        let mut args = base_args();
        args.filters = vec!["status=active".into()];
        assert!(matches!(build_query_body(&args), Err(CliError::Usage(_))));
    }

    #[test]
    fn missing_fields_is_a_usage_error() {
        let mut args = base_args();
        args.fields = None;
        assert!(matches!(build_query_body(&args), Err(CliError::Usage(_))));
    }
}
