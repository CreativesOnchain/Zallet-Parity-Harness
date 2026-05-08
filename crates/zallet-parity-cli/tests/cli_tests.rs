use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use std::fs;
use tempfile::tempdir;
use zallet_parity_testkit::MockNode;

// ── Helper ────────────────────────────────────────────────────────────────────

/// Writes a single-method manifest to a temp file and returns the path.
fn write_manifest(dir: &tempfile::TempDir, method: &str) -> std::path::PathBuf {
    let path = dir.path().join("manifest.toml");
    fs::write(&path, format!("[[methods]]\nname = \"{}\"\n", method)).unwrap();
    path
}

// ── Exit code 0: all methods match ───────────────────────────────────────────

#[tokio::test]
async fn test_cli_exit_0_on_clean_match() -> Result<(), Box<dyn std::error::Error>> {
    let upstream = MockNode::spawn().await;
    let target = MockNode::spawn().await;
    let method = "getblockchaininfo";

    upstream
        .mock_response(method, json!(null), json!({"blocks": 100}))
        .await;
    target
        .mock_response(method, json!(null), json!({"blocks": 100}))
        .await;

    let dir = tempdir()?;
    let manifest_path = write_manifest(&dir, method);
    let report_path = dir.path().join("report.json");

    Command::cargo_bin("zallet-rpc-diff")?
        .arg("run")
        .arg("--upstream-url")
        .arg(upstream.url())
        .arg("--target-url")
        .arg(target.url())
        .arg("--manifest")
        .arg(&manifest_path)
        .arg("--output")
        .arg(&report_path)
        .assert()
        .success() // exit code 0
        .stdout(predicate::str::contains("Parity check complete!"))
        .stdout(predicate::str::contains("✅ 1 match"));

    // Verify report files are written
    assert!(report_path.exists());
    let report_content = fs::read_to_string(&report_path)?;
    assert!(report_content.contains("\"matches\": 1"));

    let md_path = report_path.with_extension("md");
    assert!(md_path.exists());
    let md_content = fs::read_to_string(&md_path)?;
    assert!(md_content.contains("✅ Matches**: 1"));

    Ok(())
}

// ── Exit code 1: unexpected diff found ───────────────────────────────────────

#[tokio::test]
async fn test_cli_exit_1_on_unexpected_diff() -> Result<(), Box<dyn std::error::Error>> {
    let upstream = MockNode::spawn().await;
    let target = MockNode::spawn().await;
    let method = "getblockchaininfo";

    upstream
        .mock_response(method, json!(null), json!({"chain": "main"}))
        .await;
    target
        .mock_response(method, json!(null), json!({"chain": "test"}))
        .await;

    let dir = tempdir()?;
    let manifest_path = write_manifest(&dir, method);
    let report_path = dir.path().join("report.json");

    Command::cargo_bin("zallet-rpc-diff")?
        .arg("run")
        .arg("--upstream-url")
        .arg(upstream.url())
        .arg("--target-url")
        .arg(target.url())
        .arg("--manifest")
        .arg(&manifest_path)
        .arg("--output")
        .arg(&report_path)
        .assert()
        .failure() // exit code 1, not 0
        .code(1)
        .stdout(predicate::str::contains("❌ 1 diff"));

    Ok(())
}

// ── Exit code 1: missing method on target ────────────────────────────────────

#[tokio::test]
async fn test_cli_exit_1_on_missing_method() -> Result<(), Box<dyn std::error::Error>> {
    let upstream = MockNode::spawn().await;
    let target = MockNode::spawn().await;
    let method = "z_getaddressforaccount";

    upstream
        .mock_response(method, json!(null), json!({"address": "zs1abc"}))
        .await;
    target.mock_method_not_found(method, json!(null)).await;

    let dir = tempdir()?;
    let manifest_path = write_manifest(&dir, method);
    let report_path = dir.path().join("report.json");

    Command::cargo_bin("zallet-rpc-diff")?
        .arg("run")
        .arg("--upstream-url")
        .arg(upstream.url())
        .arg("--target-url")
        .arg(target.url())
        .arg("--manifest")
        .arg(&manifest_path)
        .arg("--output")
        .arg(&report_path)
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("🔍 1 missing"));

    Ok(())
}

// ── Exit code 2: bad manifest path ───────────────────────────────────────────

#[tokio::test]
async fn test_cli_exit_2_on_bad_manifest() -> Result<(), Box<dyn std::error::Error>> {
    let upstream = MockNode::spawn().await;
    let target = MockNode::spawn().await;

    Command::cargo_bin("zallet-rpc-diff")?
        .arg("run")
        .arg("--upstream-url")
        .arg(upstream.url())
        .arg("--target-url")
        .arg(target.url())
        .arg("--manifest")
        .arg("/nonexistent/path/manifest.toml")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("Fatal error"));

    Ok(())
}
