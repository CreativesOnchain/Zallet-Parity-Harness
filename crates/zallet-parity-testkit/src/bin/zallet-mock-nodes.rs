use serde_json::json;
use tokio::signal;
use zallet_parity_testkit::MockNode;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Zallet Parity Mock Nodes...");

    // 1. Upstream (zcashd)
    let upstream = MockNode::spawn().await;

    // 2. Target (Zallet)
    let target = MockNode::spawn().await;

    // --- Canned Responses ---

    // MATCH: getblockcount
    upstream
        .mock_response("getblockcount", json!(null), json!(2854332))
        .await;
    target
        .mock_response("getblockcount", json!(null), json!(2854332))
        .await;

    // MATCH: getconnectioncount
    upstream
        .mock_response("getconnectioncount", json!(null), json!(12))
        .await;
    target
        .mock_response("getconnectioncount", json!(null), json!(12))
        .await;

    // DIFF: getblockchaininfo (unexpected diff, e.g. "chain" changes)
    upstream
        .mock_response(
            "getblockchaininfo",
            json!(null),
            json!({"chain": "main", "blocks": 2854332}),
        )
        .await;
    target
        .mock_response(
            "getblockchaininfo",
            json!(null),
            json!({"chain": "test", "blocks": 2854332}),
        )
        .await;

    // EXPECTED DIFF: getnetworkinfo (useragent string)
    upstream
        .mock_response(
            "getnetworkinfo",
            json!(null),
            json!({"version": 1000000, "useragent": "/MagicBean:1.0.0/"}),
        )
        .await;
    target
        .mock_response(
            "getnetworkinfo",
            json!(null),
            json!({"version": 1000000, "useragent": "/Zallet:0.1.0/"}),
        )
        .await;

    // MISSING: validateaddress
    upstream
        .mock_response("validateaddress", json!(null), json!({"isvalid": false}))
        .await;
    target
        .mock_method_not_found("validateaddress", json!(null))
        .await;

    // EXPECTED MISSING: z_getaddressforaccount
    upstream
        .mock_response(
            "z_getaddressforaccount",
            json!(null),
            json!({"address": "ztestsapling..."}),
        )
        .await;
    target
        .mock_method_not_found("z_getaddressforaccount", json!(null))
        .await;

    // ERROR: getmemoryinfo (upstream fails)
    upstream
        .mock_rpc_error("getmemoryinfo", json!(null), -32603, "Internal error")
        .await;
    target
        .mock_response("getmemoryinfo", json!(null), json!({"locked": {}}))
        .await;

    // FALLBACK: Everything else returns Method Not Found
    // This allows the engine to gracefully report MISSING instead of HTTP 404 errors.
    upstream.mock_fallback().await;
    target.mock_fallback().await;

    println!("\n✅ Nodes are running!");
    println!("   Upstream (zcashd mock): {}", upstream.url());
    println!("   Target   (Zallet mock): {}", target.url());

    println!("\nTo test the parity harness, open a new terminal and run:");
    println!(
        "cargo run -q -- run --upstream-url {} --target-url {} --manifest manifest.toml --expected-diffs examples/expected_diffs.toml --output report.json",
        upstream.url(),
        target.url()
    );

    println!("\nPress Ctrl+C to shut down the mock nodes.");

    // Wait for Ctrl-C
    signal::ctrl_c().await?;
    println!("\nShutting down...");

    Ok(())
}
