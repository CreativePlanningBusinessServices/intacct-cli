//! Real-API smoke tests. Excluded from CI (all #[ignore]).
//! Run: INTACCT_LIVE_ALIAS=<alias> cargo test --test live_smoke -- --ignored --nocapture
use intacct_cli::commands::{describe, object, query};
use intacct_cli::context::context_for;

fn live_alias() -> String {
    std::env::var("INTACCT_LIVE_ALIAS")
        .expect("set INTACCT_LIVE_ALIAS to a configured account alias")
}

#[tokio::test]
#[ignore]
async fn model_service_lists_objects() {
    let context = context_for(Some(&live_alias()), None).unwrap();
    let result =
        describe::list_resources(&context.client, "object", Some("^accounts-payable/".into()))
            .await
            .unwrap();
    println!("{result}");
}

#[tokio::test]
#[ignore]
async fn query_reads_vendors() {
    let context = context_for(Some(&live_alias()), None).unwrap();
    let body = serde_json::json!({
        "object": "accounts-payable/vendor",
        "fields": ["key", "id", "name"],
        "size": 5
    });
    let result = query::run(&context.client, body, false).await.unwrap();
    println!("{result}");
}

#[tokio::test]
#[ignore]
async fn object_list_pages() {
    let context = context_for(Some(&live_alias()), None).unwrap();
    let result = object::list(
        &context.client,
        "general-ledger/account",
        None,
        Some(5),
        false,
    )
    .await
    .unwrap();
    println!("{result}");
}
