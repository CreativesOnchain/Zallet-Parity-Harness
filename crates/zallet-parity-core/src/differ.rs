//! Structured JSON diff walker.
//!
//! Compares two `serde_json::Value` trees recursively and returns a list
//! of leaf-level differences as [`DiffEntry`] items, each identified by
//! its JSON Pointer path (RFC 6901).
//!
//! This replaces the plain `diff_message: String` from `assert-json-diff`
//! with a structured, serializable output suitable for the parity report.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Public types ──────────────────────────────────────────────────────────────

/// A single leaf-level difference between upstream and target.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffEntry {
    /// RFC 6901 JSON Pointer to the location of the difference.
    /// Example: `/softforks/0/id`, `/chain`
    pub path: String,
    /// The value from the upstream (zcashd) response.
    pub upstream: Value,
    /// The value from the target (Zallet) response.
    pub target: Value,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Recursively compares `upstream` and `target`, collecting all leaf-level
/// differences into a `Vec<DiffEntry>`.
///
/// - Object keys present in one but absent from the other are reported.
/// - Array elements present in one but absent from the other are reported.
/// - Scalar differences are reported at the exact path.
pub fn diff_values(upstream: &Value, target: &Value) -> Vec<DiffEntry> {
    let mut entries = Vec::new();
    diff_recursive(upstream, target, "", &mut entries);
    entries
}

// ── Recursive walker ──────────────────────────────────────────────────────────

fn diff_recursive(upstream: &Value, target: &Value, path: &str, out: &mut Vec<DiffEntry>) {
    match (upstream, target) {
        (Value::Object(u_map), Value::Object(t_map)) => {
            diff_objects(u_map, t_map, path, out);
        }
        (Value::Array(u_arr), Value::Array(t_arr)) => {
            diff_arrays(u_arr, t_arr, path, out);
        }
        (u, t) if u != t => {
            out.push(DiffEntry::at(path, u.clone(), t.clone()));
        }
        _ => {} // Equal values — no diff
    }
}

// ── Branch helpers ────────────────────────────────────────────────────────────

/// Diffs two JSON objects key-by-key.
///
/// - Keys present in upstream but absent from target are reported as `target = Null`.
/// - Keys present in target but absent from upstream are reported as `upstream = Null`.
/// - Keys present in both are recursed into.
fn diff_objects(
    u_map: &serde_json::Map<String, Value>,
    t_map: &serde_json::Map<String, Value>,
    path: &str,
    out: &mut Vec<DiffEntry>,
) {
    for (key, u_val) in u_map {
        let child = child_path(path, &escape_token(key));
        match t_map.get(key) {
            Some(t_val) => diff_recursive(u_val, t_val, &child, out),
            None => out.push(DiffEntry::at(&child, u_val.clone(), Value::Null)),
        }
    }

    for (key, t_val) in t_map {
        if !u_map.contains_key(key) {
            let child = child_path(path, &escape_token(key));
            out.push(DiffEntry::at(&child, Value::Null, t_val.clone()));
        }
    }
}

/// Diffs two JSON arrays element-by-element.
///
/// Iterates up to `max(upstream.len(), target.len())`. Elements present in
/// only one array are reported as missing (`Null`) on the other side.
fn diff_arrays(u_arr: &[Value], t_arr: &[Value], path: &str, out: &mut Vec<DiffEntry>) {
    let max_len = u_arr.len().max(t_arr.len());

    for i in 0..max_len {
        let child = child_path(path, &i.to_string());
        match (u_arr.get(i), t_arr.get(i)) {
            (Some(u), Some(t)) => diff_recursive(u, t, &child, out),
            (Some(u), None) => out.push(DiffEntry::at(&child, u.clone(), Value::Null)),
            (None, Some(t)) => out.push(DiffEntry::at(&child, Value::Null, t.clone())),
            (None, None) => {}
        }
    }
}

// ── Path utilities ────────────────────────────────────────────────────────────

/// Builds a child JSON Pointer path by appending a token to a parent path.
///
/// An empty parent path represents the document root; the result is `"/token"`.
/// A non-empty parent path results in `"<parent>/token"`.
fn child_path(parent: &str, token: &str) -> String {
    format!("{}/{}", parent, token)
}

/// Returns the canonical path for a diff, normalising an empty root path to `"/"`.
///
/// Per RFC 6901, the root document is represented by an empty string, but for
/// human-readable output we use `"/"` to make it obvious.
impl DiffEntry {
    fn at(path: &str, upstream: Value, target: Value) -> Self {
        DiffEntry {
            path: if path.is_empty() {
                "/".to_string()
            } else {
                path.to_string()
            },
            upstream,
            target,
        }
    }
}

/// Escapes a JSON Pointer token as per RFC 6901:
/// `~` → `~0`, `/` → `~1`
fn escape_token(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_equal_values_produce_no_diff() {
        let a = json!({"chain": "main", "blocks": 100});
        let b = json!({"chain": "main", "blocks": 100});
        assert!(diff_values(&a, &b).is_empty());
    }

    #[test]
    fn test_scalar_diff_at_root_field() {
        let a = json!({"chain": "main"});
        let b = json!({"chain": "test"});
        let diffs = diff_values(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "/chain");
        assert_eq!(diffs[0].upstream, json!("main"));
        assert_eq!(diffs[0].target, json!("test"));
    }

    #[test]
    fn test_nested_diff_has_correct_path() {
        let a = json!({"status": {"synced": true, "blocks": 100}});
        let b = json!({"status": {"synced": false, "blocks": 100}});
        let diffs = diff_values(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "/status/synced");
    }

    #[test]
    fn test_missing_key_in_target_is_reported() {
        let a = json!({"chain": "main", "extra": "field"});
        let b = json!({"chain": "main"});
        let diffs = diff_values(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "/extra");
        assert_eq!(diffs[0].target, Value::Null);
    }

    #[test]
    fn test_extra_key_in_target_is_reported() {
        let a = json!({"chain": "main"});
        let b = json!({"chain": "main", "extra": "field"});
        let diffs = diff_values(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "/extra");
        assert_eq!(diffs[0].upstream, Value::Null);
    }

    #[test]
    fn test_array_element_diff_path_includes_index() {
        let a = json!({"softforks": [{"id": "csv"}, {"id": "segwit"}]});
        let b = json!({"softforks": [{"id": "csv"}, {"id": "taproot"}]});
        let diffs = diff_values(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "/softforks/1/id");
    }

    #[test]
    fn test_key_with_special_chars_is_escaped() {
        let a = json!({"a/b": 1});
        let b = json!({"a/b": 2});
        let diffs = diff_values(&a, &b);
        assert_eq!(diffs.len(), 1);
        // '/' in key must be escaped as '~1'
        assert_eq!(diffs[0].path, "/a~1b");
    }

    #[test]
    fn test_multiple_diffs_all_reported() {
        let a = json!({"x": 1, "y": 2, "z": 3});
        let b = json!({"x": 1, "y": 99, "z": 99});
        let diffs = diff_values(&a, &b);
        assert_eq!(diffs.len(), 2);
        let paths: Vec<&str> = diffs.iter().map(|d| d.path.as_str()).collect();
        assert!(paths.contains(&"/y"));
        assert!(paths.contains(&"/z"));
    }

    #[test]
    fn test_root_scalar_diff_reports_slash_path() {
        let diffs = diff_values(&json!(1), &json!(2));
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "/");
    }

    #[test]
    fn test_array_length_mismatch_reports_extra_element() {
        let a = json!([1, 2, 3]);
        let b = json!([1, 2]);
        let diffs = diff_values(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "/2");
        assert_eq!(diffs[0].target, Value::Null);
    }
}
