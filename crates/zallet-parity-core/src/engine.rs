use crate::Error;
use crate::client::RpcClient;
use crate::differ::{DiffEntry, diff_values};
use crate::expected_diffs::{ExpectedDiffEntry, ExpectedDiffs};
use crate::manifest::MethodEntry;
use crate::normalizer::{normalize, parse_ignore_paths};
use jsonptr::PointerBuf;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// Default maximum number of concurrent RPC calls.
pub const DEFAULT_CONCURRENCY: usize = 8;

/// JSON-RPC "method not found" error code per spec.
const METHOD_NOT_FOUND_CODE: i32 = -32601;

// ── Result type ───────────────────────────────────────────────────────────────

/// The result of a single parity check.
#[derive(Debug, Clone)]
pub enum ParityResult {
    /// Both endpoints returned identical data (after normalization).
    Match,
    /// Both endpoints returned data, but the normalized values differ —
    /// and this difference was NOT anticipated by the expected-diffs file.
    Diff {
        /// Structured list of leaf-level differences (with JSON Pointer paths).
        diff_entries: Vec<DiffEntry>,
    },
    /// The diff was found in the expected-diffs file — it is a known,
    /// intentional divergence. Visible in the report but not a blocker.
    ExpectedDiff {
        diff_entries: Vec<DiffEntry>,
        reason: String,
    },
    /// One or both endpoints returned -32601 "method not found",
    /// and it was NOT listed as `expected_missing` in the config.
    Missing { method: String },
    /// The method returned -32601 and was declared `expected_missing = true`
    /// in `expected_diffs.toml`. Not a blocker.
    ExpectedMissing { method: String, reason: String },
    /// A transport failure or non-missing RPC error occurred.
    Error(String),
}

// ── Engine ────────────────────────────────────────────────────────────────────

/// The engine responsible for executing the parity suite.
pub struct ParityEngine {
    upstream: RpcClient,
    target: RpcClient,
}

impl ParityEngine {
    pub fn new(upstream: RpcClient, target: RpcClient) -> Self {
        Self { upstream, target }
    }

    /// Runs the parity checks for all methods defined in the manifest.
    ///
    /// At most `concurrency` RPC pairs are in-flight at once; set to
    /// [`DEFAULT_CONCURRENCY`] for typical use.
    /// The normalization pipeline (key-sort + ignore-paths) is applied
    /// before comparison. If a diff is found and it matches an entry in
    /// `expected_diffs`, it is classified as [`ParityResult::ExpectedDiff`]
    /// instead of [`ParityResult::Diff`].
    pub async fn run_all(
        &self,
        methods: Vec<MethodEntry>,
        expected_diffs: &ExpectedDiffs,
        concurrency: usize,
        save_raw: bool,
    ) -> Vec<(String, ParityResult)> {
        let expected_entries: Vec<_> = expected_diffs.expected.clone();
        let sem = Arc::new(Semaphore::new(concurrency.max(1)));

        let mut set = spawn_tasks(
            &self.upstream,
            &self.target,
            methods,
            expected_entries,
            sem,
            save_raw,
        );
        collect_results(&mut set).await
    }
}

// ── Task spawning ─────────────────────────────────────────────────────────────

/// Spawns one async task per method, bounded by a shared semaphore.
fn spawn_tasks(
    upstream: &RpcClient,
    target: &RpcClient,
    methods: Vec<MethodEntry>,
    expected_entries: Vec<ExpectedDiffEntry>,
    sem: Arc<Semaphore>,
    save_raw: bool,
) -> JoinSet<(String, ParityResult)> {
    let mut set = JoinSet::new();

    for entry in methods {
        let upstream = upstream.clone();
        let target = target.clone();
        let expected = expected_entries.clone();
        let sem = Arc::clone(&sem);

        set.spawn(async move {
            // Acquire a permit; released automatically when `_permit` is dropped.
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            run_single_method(upstream, target, entry, expected, save_raw).await
        });
    }

    set
}

/// Awaits all spawned tasks and collects their results, logging any panics.
async fn collect_results(set: &mut JoinSet<(String, ParityResult)>) -> Vec<(String, ParityResult)> {
    let mut results = Vec::new();
    while let Some(join_result) = set.join_next().await {
        match join_result {
            Ok(tagged) => results.push(tagged),
            Err(e) => tracing::error!("Parity task panicked: {}", e),
        }
    }
    results
}

// ── Single-method execution ───────────────────────────────────────────────────

/// Executes the parity check for one method end-to-end.
///
/// Phases: parse ignore paths → call both endpoints → normalize → diff → classify.
async fn run_single_method(
    upstream: RpcClient,
    target: RpcClient,
    entry: MethodEntry,
    expected_entries: Vec<ExpectedDiffEntry>,
    save_raw: bool,
) -> (String, ParityResult) {
    let method_name = entry.name.clone();
    let params = entry.params.clone().unwrap_or(Value::Null);
    let ignore_paths = resolve_ignore_paths(&method_name, &entry.ignore_paths);

    let res_u = upstream.call(&method_name, params.clone()).await;
    let res_t = target.call(&method_name, params).await;

    let u_str = match &res_u {
        Ok(v) => serde_json::to_string_pretty(v).unwrap_or_else(|_| format!("{:?}", v)),
        Err(e) => format!("Error: {}", e),
    };
    let t_str = match &res_t {
        Ok(v) => serde_json::to_string_pretty(v).unwrap_or_else(|_| format!("{:?}", v)),
        Err(e) => format!("Error: {}", e),
    };

    let parity = classify_rpc_results(
        res_u,
        res_t,
        &method_name,
        &ignore_paths,
        &expected_entries,
    );

    if save_raw && !matches!(parity, ParityResult::Match) {
        let _ = std::fs::create_dir_all("reports/raw");
        let _ = std::fs::write(format!("reports/raw/upstream_{}.json", method_name), u_str);
        let _ = std::fs::write(format!("reports/raw/target_{}.json", method_name), t_str);
    }

    (method_name, parity)
}

/// Parses and returns the compiled ignore paths for a method.
///
/// Logs a warning and returns an empty list if any path is invalid,
/// so a misconfigured manifest never silently blocks the run.
fn resolve_ignore_paths(method_name: &str, raw: &[String]) -> Vec<PointerBuf> {
    match parse_ignore_paths(raw) {
        Ok(paths) => paths,
        Err(e) => {
            tracing::warn!("Invalid ignore path for '{}': {}", method_name, e);
            vec![]
        }
    }
}

// ── Classification ────────────────────────────────────────────────────────────

/// Maps the pair of RPC call outcomes to a [`ParityResult`].
///
/// The happy path (both `Ok`) delegates to [`classify_diff`].
/// All error cases delegate to [`classify_error_pair`].
fn classify_rpc_results(
    res_u: Result<Value, Error>,
    res_t: Result<Value, Error>,
    method_name: &str,
    ignore_paths: &[PointerBuf],
    expected_entries: &[ExpectedDiffEntry],
) -> ParityResult {
    match (res_u, res_t) {
        (Ok(u), Ok(t)) => {
            let u_norm = normalize(u, ignore_paths);
            let t_norm = normalize(t, ignore_paths);
            let diff_entries = diff_values(&u_norm, &t_norm);
            classify_diff(diff_entries, method_name, expected_entries)
        }
        (err_u, err_t) => classify_error_pair(err_u, err_t, method_name, expected_entries),
    }
}

/// Classifies a successful (both endpoints responded) comparison result.
///
/// Returns `Match` if there are no diffs, `ExpectedDiff` if all diffs are
/// covered by the expected-diffs file, or `Diff` otherwise.
fn classify_diff(
    diff_entries: Vec<DiffEntry>,
    method_name: &str,
    expected_entries: &[ExpectedDiffEntry],
) -> ParityResult {
    if diff_entries.is_empty() {
        return ParityResult::Match;
    }

    let actual_paths: Vec<&str> = diff_entries.iter().map(|e| e.path.as_str()).collect();

    if let Some(expected) = find_expected_entry(method_name, &actual_paths, expected_entries) {
        ParityResult::ExpectedDiff {
            diff_entries,
            reason: expected.reason.clone(),
        }
    } else {
        ParityResult::Diff { diff_entries }
    }
}

/// Searches for an expected-diff entry that covers all actual diff paths.
///
/// A method-level entry (`diff_paths` is empty) matches any set of paths.
/// A field-level entry matches only when every actual path is prefixed by
/// at least one of the declared expected paths.
fn find_expected_entry<'a>(
    method_name: &str,
    actual_paths: &[&str],
    expected_entries: &'a [ExpectedDiffEntry],
) -> Option<&'a ExpectedDiffEntry> {
    expected_entries.iter().find(|ee| {
        if ee.method != method_name {
            return false;
        }
        // Method-level entry: any diff on this method is expected.
        if ee.diff_paths.is_empty() {
            return true;
        }
        // Field-level entry: every actual path must be covered.
        actual_paths
            .iter()
            .all(|p| ee.diff_paths.iter().any(|ep| p.starts_with(ep.as_str())))
    })
}

/// Classifies a result pair where at least one side returned an error.
///
/// If the error is method-not-found (-32601) and `expected_entries` contains
/// an `expected_missing = true` entry for this method, returns `ExpectedMissing`.
/// Otherwise returns `Missing` (for method-not-found) or `Error`.
fn classify_error_pair(
    res_u: Result<Value, Error>,
    res_t: Result<Value, Error>,
    method_name: &str,
    expected_entries: &[ExpectedDiffEntry],
) -> ParityResult {
    let is_missing = matches!((&res_u, &res_t),
        (Err(e_u), Err(e_t)) if is_method_not_found(e_u) && is_method_not_found(e_t)
    ) || matches!((&res_u, &res_t),
        (Err(e), _) | (_, Err(e)) if is_method_not_found(e)
    );

    if is_missing {
        // Check for expected_missing declaration in config.
        if let Some(entry) = expected_entries
            .iter()
            .find(|e| e.method == method_name && e.expected_missing)
        {
            return ParityResult::ExpectedMissing {
                method: method_name.to_string(),
                reason: entry.reason.clone(),
            };
        }
        return ParityResult::Missing {
            method: method_name.to_string(),
        };
    }

    match (&res_u, &res_t) {
        // Upstream transport/RPC error
        (Err(e), _) => ParityResult::Error(format!("Upstream error: {}", e)),
        // Target transport/RPC error
        (_, Err(e)) => ParityResult::Error(format!("Target error: {}", e)),
        // Both Ok — should not reach here.
        (Ok(_), Ok(_)) => unreachable!("classify_error_pair called with two Ok results"),
    }
}

/// Returns `true` if the given error represents a "method not found" response.
fn is_method_not_found(err: &Error) -> bool {
    match err {
        Error::JsonRpc(e) => {
            let s = e.to_string();
            s.contains(&METHOD_NOT_FOUND_CODE.to_string())
                || s.to_lowercase().contains("method not found")
        }
        _ => false,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expected_diffs::{ExpectedDiffEntry, ExpectedDiffs};
    use crate::manifest::MethodEntry;
    use serde_json::json;
    use zallet_parity_testkit::MockNode;

    fn entry(name: &str) -> MethodEntry {
        MethodEntry {
            name: name.to_string(),
            params: None,
            ignore_paths: vec![],
            tags: vec![],
        }
    }

    fn entry_with_ignore(name: &str, paths: Vec<&str>) -> MethodEntry {
        MethodEntry {
            name: name.to_string(),
            params: None,
            ignore_paths: paths.into_iter().map(String::from).collect(),
            tags: vec![],
        }
    }

    fn entry_with_params(name: &str, params: serde_json::Value) -> MethodEntry {
        MethodEntry {
            name: name.to_string(),
            params: Some(params),
            ignore_paths: vec![],
            tags: vec![],
        }
    }

    fn no_expected() -> ExpectedDiffs {
        ExpectedDiffs::none()
    }

    // ── MATCH ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_parity_match() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_match";
        let resp = json!({"blocks": 100, "chain": "main"});
        u.mock_response(method, json!(null), resp.clone()).await;
        t.mock_response(method, json!(null), resp).await;
        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(vec![entry(method)], &no_expected(), DEFAULT_CONCURRENCY, false)
            .await;
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].1, ParityResult::Match));
    }

    // ── MATCH via normalization ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_parity_ordering_only_diff_is_match_after_normalization() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_ordering";
        u.mock_response(method, json!(null), json!({"z": 1, "a": 2, "m": 3}))
            .await;
        t.mock_response(method, json!(null), json!({"a": 2, "m": 3, "z": 1}))
            .await;
        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(vec![entry(method)], &no_expected(), DEFAULT_CONCURRENCY, false)
            .await;
        assert_eq!(results.len(), 1);
        assert!(
            matches!(results[0].1, ParityResult::Match),
            "ordering-only diff should be MATCH, got: {:?}",
            results[0].1
        );
    }

    // ── DIFF ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_parity_diff_returns_structured_paths() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_diff";
        u.mock_response(method, json!(null), json!({"chain": "main", "blocks": 100}))
            .await;
        t.mock_response(method, json!(null), json!({"chain": "test", "blocks": 100}))
            .await;
        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(vec![entry(method)], &no_expected(), DEFAULT_CONCURRENCY, false)
            .await;
        assert_eq!(results.len(), 1);
        if let ParityResult::Diff { diff_entries } = &results[0].1 {
            assert_eq!(diff_entries.len(), 1);
            assert_eq!(diff_entries[0].path, "/chain");
        } else {
            panic!("expected Diff, got {:?}", results[0].1);
        }
    }

    // ── EXPECTED_DIFF (method-level) ──────────────────────────────────────────

    #[tokio::test]
    async fn test_parity_expected_diff_method_level_is_labeled() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_expected_diff";
        u.mock_response(method, json!(null), json!({"version": "zcashd/4.7.0"}))
            .await;
        t.mock_response(method, json!(null), json!({"version": "zallet/0.1.0"}))
            .await;

        let expected = ExpectedDiffs {
            expected: vec![ExpectedDiffEntry {
                method: method.to_string(),
                reason: "Zallet reports a different version string.".to_string(),
                diff_paths: vec![], // method-level: any diff is expected
                expected_missing: false,
            }],
        };

        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(vec![entry(method)], &expected, DEFAULT_CONCURRENCY, false)
            .await;
        assert_eq!(results.len(), 1);
        if let ParityResult::ExpectedDiff { reason, .. } = &results[0].1 {
            assert!(reason.contains("version"));
        } else {
            panic!("expected ExpectedDiff, got {:?}", results[0].1);
        }
    }

    // ── EXPECTED_DIFF (field-level) covers exact paths ────────────────────────

    #[tokio::test]
    async fn test_parity_expected_diff_field_level_covered() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_field_expected";
        u.mock_response(
            method,
            json!(null),
            json!({"chain": "main", "softforks": [{"id": "csv"}]}),
        )
        .await;
        t.mock_response(
            method,
            json!(null),
            json!({"chain": "main", "softforks": []}),
        )
        .await;

        let expected = ExpectedDiffs {
            expected: vec![ExpectedDiffEntry {
                method: method.to_string(),
                reason: "Zallet omits softforks field.".to_string(),
                diff_paths: vec!["/softforks".to_string()],
                expected_missing: false,
            }],
        };

        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(vec![entry(method)], &expected, DEFAULT_CONCURRENCY, false)
            .await;
        assert_eq!(results.len(), 1);
        assert!(
            matches!(results[0].1, ParityResult::ExpectedDiff { .. }),
            "covered field diff should be ExpectedDiff, got: {:?}",
            results[0].1
        );
    }

    // ── DIFF when only some paths are expected ────────────────────────────────

    #[tokio::test]
    async fn test_parity_unexpected_diff_when_extra_path_differs() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_partial_expected";
        u.mock_response(
            method,
            json!(null),
            json!({"chain": "main", "softforks": [{"id": "csv"}]}),
        )
        .await;
        t.mock_response(
            method,
            json!(null),
            json!({"chain": "test", "softforks": []}),
        )
        .await;

        let expected = ExpectedDiffs {
            expected: vec![ExpectedDiffEntry {
                method: method.to_string(),
                reason: "Only softforks is expected.".to_string(),
                diff_paths: vec!["/softforks".to_string()],
                expected_missing: false,
            }],
        };

        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(vec![entry(method)], &expected, DEFAULT_CONCURRENCY, false)
            .await;
        assert_eq!(results.len(), 1);
        assert!(
            matches!(results[0].1, ParityResult::Diff { .. }),
            "partial coverage should remain DIFF, got: {:?}",
            results[0].1
        );
    }

    // ── MATCH via ignore_paths ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_parity_ignore_path_suppresses_diff() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_ignore";
        u.mock_response(
            method,
            json!(null),
            json!({"chain": "main", "volatile": 999}),
        )
        .await;
        t.mock_response(
            method,
            json!(null),
            json!({"chain": "main", "volatile": 888}),
        )
        .await;
        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(
                vec![entry_with_ignore(method, vec!["/volatile"])],
                &no_expected(),
                DEFAULT_CONCURRENCY, false)
            .await;
        assert_eq!(results.len(), 1);
        assert!(
            matches!(results[0].1, ParityResult::Match),
            "ignored path diff should be MATCH, got: {:?}",
            results[0].1
        );
    }

    // ── MISSING ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_parity_missing_on_target() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_missing";
        u.mock_response(method, json!(null), json!({"ok": true}))
            .await;
        t.mock_method_not_found(method, json!(null)).await;
        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(vec![entry(method)], &no_expected(), DEFAULT_CONCURRENCY, false)
            .await;
        assert_eq!(results.len(), 1);
        assert!(
            matches!(&results[0].1, ParityResult::Missing { method: m } if m == method),
            "expected Missing, got {:?}",
            results[0].1
        );
    }

    // ── ERROR ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_parity_error_on_upstream() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_error";
        u.mock_rpc_error(method, json!(null), -32603, "Internal server error")
            .await;
        t.mock_response(method, json!(null), json!({"ok": true}))
            .await;
        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(vec![entry(method)], &no_expected(), DEFAULT_CONCURRENCY, false)
            .await;
        assert_eq!(results.len(), 1);
        assert!(
            matches!(&results[0].1, ParityResult::Error(msg) if msg.contains("Upstream error")),
            "expected Error, got {:?}",
            results[0].1
        );
    }

    // ── EXPECTED_MISSING ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_parity_expected_missing_is_classified_correctly() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_expected_missing";
        u.mock_response(method, json!(null), json!({"ok": true}))
            .await;
        t.mock_method_not_found(method, json!(null)).await;

        let expected = ExpectedDiffs {
            expected: vec![ExpectedDiffEntry {
                method: method.to_string(),
                reason: "Not yet implemented in Zallet.".to_string(),
                diff_paths: vec![],
                expected_missing: true,
            }],
        };

        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(vec![entry(method)], &expected, DEFAULT_CONCURRENCY, false)
            .await;
        assert_eq!(results.len(), 1);
        assert!(
            matches!(
                &results[0].1,
                ParityResult::ExpectedMissing { method: m, reason: r }
                    if m == method && r.contains("Not yet implemented")
            ),
            "expected ExpectedMissing, got {:?}",
            results[0].1
        );
    }

    #[tokio::test]
    async fn test_parity_missing_without_expected_flag_stays_missing() {
        // Same setup, but NO expected_missing entry — should remain MISSING.
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_unexpected_missing";
        u.mock_response(method, json!(null), json!({"ok": true}))
            .await;
        t.mock_method_not_found(method, json!(null)).await;

        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(vec![entry(method)], &no_expected(), DEFAULT_CONCURRENCY, false)
            .await;
        assert!(
            matches!(&results[0].1, ParityResult::Missing { .. }),
            "expected Missing (no expected_missing entry), got {:?}",
            results[0].1
        );
    }

    // ── Params shape tests ────────────────────────────────────────────────────
    // Verify that the client sends the correct params wire format.
    // The MockNode only responds if params match exactly, so a wrong encoding
    // would return ERROR instead of MATCH.

    /// `params = null` (or absent) must send empty params — NOT `[null]`.
    #[tokio::test]
    async fn test_params_null_sends_empty() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_null_params";
        // testkit matches the first element; json!(null) means no-params sentinel
        u.mock_response(method, json!(null), json!({"ok": true}))
            .await;
        t.mock_response(method, json!(null), json!({"ok": true}))
            .await;
        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(vec![entry(method)], &no_expected(), DEFAULT_CONCURRENCY, false)
            .await;
        assert!(
            matches!(results[0].1, ParityResult::Match),
            "null-params method should MATCH, got: {:?}",
            results[0].1
        );
    }

    /// `params = [""]` must send `[""]` — NOT `[[""]]`.
    #[tokio::test]
    async fn test_params_array_is_spread() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_array_params";
        // testkit matches first element of the params array sent over the wire.
        // If params were double-wrapped it would be [""] not "" -> mock no-match -> ERROR.
        u.mock_response(method, json!(""), json!({"ok": true}))
            .await;
        t.mock_response(method, json!(""), json!({"ok": true}))
            .await;
        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(
                vec![entry_with_params(method, json!([""]))],
                &no_expected(),
                DEFAULT_CONCURRENCY, false)
            .await;
        assert!(
            matches!(results[0].1, ParityResult::Match),
            "array-params method should MATCH (not ERROR), got: {:?}",
            results[0].1
        );
    }

    /// `params = "scalar"` must send `["scalar"]` (single positional arg).
    #[tokio::test]
    async fn test_params_scalar_is_wrapped() {
        let u = MockNode::spawn().await;
        let t = MockNode::spawn().await;
        let method = "test_scalar_params";
        u.mock_response(method, json!("myaddr"), json!({"ok": true}))
            .await;
        t.mock_response(method, json!("myaddr"), json!({"ok": true}))
            .await;
        let engine = ParityEngine::new(
            RpcClient::new(&u.url()).unwrap(),
            RpcClient::new(&t.url()).unwrap(),
        );
        let results = engine
            .run_all(
                vec![entry_with_params(method, json!("myaddr"))],
                &no_expected(),
                DEFAULT_CONCURRENCY, false)
            .await;
        assert!(
            matches!(results[0].1, ParityResult::Match),
            "scalar-params method should MATCH (not ERROR), got: {:?}",
            results[0].1
        );
    }

    // ── Deterministic report ordering ─────────────────────────────────────────

    /// Two calls to FinalReport::new() with the same input must produce
    /// bit-for-bit identical JSON for the `details` block (BTreeMap order).
    #[test]
    fn test_report_ordering_is_deterministic() {
        use crate::report::FinalReport;

        let make_results = || {
            vec![
                ("z_method".to_string(), ParityResult::Match),
                ("a_method".to_string(), ParityResult::Match),
                (
                    "m_method".to_string(),
                    ParityResult::Diff {
                        diff_entries: vec![],
                    },
                ),
            ]
        };

        let r1 = FinalReport::new(make_results());
        let r2 = FinalReport::new(make_results());

        let j1 = serde_json::to_string(&r1).unwrap();
        let j2 = serde_json::to_string(&r2).unwrap();

        // Strip the timestamp (which changes) and compare the structural part.
        fn after_summary(s: &str) -> &str {
            s.find("\"summary\"").map(|i| &s[i..]).unwrap_or(s)
        }
        assert_eq!(
            after_summary(&j1),
            after_summary(&j2),
            "details JSON must be identical across runs"
        );

        // Verify keys appear in lexicographic order.
        let val: serde_json::Value = serde_json::from_str(&j1).unwrap();
        let keys: Vec<String> = val["details"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "details keys must be in lexicographic order");
    }
}
