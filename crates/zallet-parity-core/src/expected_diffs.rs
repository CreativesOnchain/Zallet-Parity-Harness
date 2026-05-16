//! Expected-differences file format and loader.
//!
//! An `expected_diffs.toml` file allows operators to annotate known,
//! intentional divergences between zcashd and Zallet. These entries
//! prevent known diffs from masking newly-introduced regressions.
//!
//! # File Format
//!
//! ```toml
//! # Intentional divergences between zcashd and Zallet.
//! # Each entry marks a known difference so that it is labeled
//! # EXPECTED_DIFF in the report instead of DIFF.
//!
//! [[expected]]
//! method = "getblockchaininfo"
//! reason = "Zallet intentionally omits the 'softforks' field."
//! # Optional: if omitted, the entire method result is considered an expected diff.
//! # If provided, only diffs at these JSON Pointer paths are expected.
//! diff_paths = ["/softforks"]
//!
//! [[expected]]
//! method = "getnetworkinfo"
//! reason = "Zallet reports a different version string."
//! diff_paths = ["/version", "/subversion"]
//!
//! # Methods not yet implemented by Zallet: set expected_missing = true.
//! # The engine will classify these as EXPECTED_MISSING instead of MISSING.
//! [[expected]]
//! method = "z_getaddressforaccount"
//! reason = "Not yet implemented in Zallet."
//! expected_missing = true
//! ```

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single expected-difference entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExpectedDiffEntry {
    /// The RPC method name this entry applies to.
    pub method: String,
    /// Human-readable reason for the known divergence.
    pub reason: String,
    /// Optional JSON Pointer paths (RFC 6901) where the diff is expected.
    ///
    /// - If `None` or empty: any diff on this method is considered expected.
    /// - If non-empty: only diffs at these specific paths are considered expected;
    ///   diffs at other paths will still be classified as `DIFF`.
    #[serde(default)]
    pub diff_paths: Vec<String>,
    /// If `true`, this method is expected to return "method not found" (`-32601`)
    /// on the target (Zallet). The engine will classify it as `EXPECTED_MISSING`
    /// rather than `MISSING`. Set this for methods Zallet has not yet implemented.
    #[serde(default)]
    pub expected_missing: bool,
}

/// The top-level expected-differences file structure.
#[derive(Debug, Deserialize)]
pub struct ExpectedDiffs {
    #[serde(default)]
    pub expected: Vec<ExpectedDiffEntry>,
}

impl ExpectedDiffs {
    /// Loads an expected-differences file from disk.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Manifest(format!("Failed to read expected-diffs file: {}", e)))?;
        toml::from_str(&content)
            .map_err(|e| Error::Manifest(format!("Failed to parse expected-diffs file: {}", e)))
    }

    /// Returns an empty set (no expected differences).
    pub fn none() -> Self {
        Self { expected: vec![] }
    }

    /// Checks whether the given method+paths combination is covered by an
    /// expected-difference entry.
    ///
    /// Returns the matching [`ExpectedDiffEntry`] if found, or `None` if
    /// the diff should be treated as unexpected.
    ///
    /// See [`find_covering_entry`] for the matching rules.
    pub fn is_expected(
        &self,
        method: &str,
        actual_diff_paths: &[String],
    ) -> Option<&ExpectedDiffEntry> {
        find_covering_entry(method, actual_diff_paths, &self.expected)
    }

    /// Returns the `expected_missing` entry for `method`, if one exists.
    ///
    /// Use this to classify a `MISSING` result as `EXPECTED_MISSING` when
    /// the operator has declared that Zallet has not yet implemented the method.
    pub fn is_expected_missing(&self, method: &str) -> Option<&ExpectedDiffEntry> {
        self.expected
            .iter()
            .find(|e| e.method == method && e.expected_missing)
    }
}

// ── Matching logic ────────────────────────────────────────────────────────────

/// Searches `entries` for one that covers all `actual_diff_paths` for `method`.
///
/// Matching rules:
/// - **Method-level** (`diff_paths` is empty): any diff on this method is expected.
/// - **Field-level** (`diff_paths` is non-empty): the entry matches only when
///   *every* actual path starts with at least one of the declared expected paths.
///
/// Returns `None` if no entry covers the combination.
pub fn find_covering_entry<'a>(
    method: &str,
    actual_diff_paths: &[String],
    entries: &'a [ExpectedDiffEntry],
) -> Option<&'a ExpectedDiffEntry> {
    entries.iter().find(|entry| {
        if entry.method != method {
            return false;
        }
        if entry.diff_paths.is_empty() {
            // Method-level: any diff on this method is expected.
            return true;
        }
        // Field-level: every actual diff path must be covered.
        actual_diff_paths
            .iter()
            .all(|p| entry.diff_paths.iter().any(|ep| p.starts_with(ep.as_str())))
    })
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn load_from_str(toml: &str) -> ExpectedDiffs {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "{}", toml).unwrap();
        ExpectedDiffs::load(file.path()).unwrap()
    }

    // ── Parsing ───────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_method_level_entry() {
        let ed = load_from_str(
            r#"
[[expected]]
method = "getnetworkinfo"
reason = "Intentional difference in version string."
"#,
        );
        assert_eq!(ed.expected.len(), 1);
        assert_eq!(ed.expected[0].method, "getnetworkinfo");
        assert!(ed.expected[0].diff_paths.is_empty());
    }

    #[test]
    fn test_parse_field_level_entry() {
        let ed = load_from_str(
            r#"
[[expected]]
method = "getblockchaininfo"
reason = "Zallet omits softforks."
diff_paths = ["/softforks"]
"#,
        );
        assert_eq!(ed.expected[0].diff_paths, vec!["/softforks"]);
    }

    #[test]
    fn test_parse_empty_file_is_valid() {
        let ed = load_from_str("");
        assert!(ed.expected.is_empty());
    }

    #[test]
    fn test_parse_multiple_entries() {
        let ed = load_from_str(
            r#"
[[expected]]
method = "getnetworkinfo"
reason = "Version diff."

[[expected]]
method = "getwalletinfo"
reason = "Balance diff."
diff_paths = ["/balance"]
"#,
        );
        assert_eq!(ed.expected.len(), 2);
    }

    // ── is_expected ───────────────────────────────────────────────────────────

    #[test]
    fn test_method_level_entry_matches_any_diff_path() {
        let ed = load_from_str(
            r#"
[[expected]]
method = "getnetworkinfo"
reason = "Any diff is expected."
"#,
        );
        let actual_paths = vec!["/version".to_string(), "/subversion".to_string()];
        assert!(ed.is_expected("getnetworkinfo", &actual_paths).is_some());
    }

    #[test]
    fn test_field_level_entry_matches_covered_paths() {
        let ed = load_from_str(
            r#"
[[expected]]
method = "getblockchaininfo"
reason = "Softforks omitted."
diff_paths = ["/softforks"]
"#,
        );
        let actual_paths = vec!["/softforks/0/id".to_string(), "/softforks/1/id".to_string()];
        // Both paths start with /softforks — fully covered
        assert!(ed.is_expected("getblockchaininfo", &actual_paths).is_some());
    }

    #[test]
    fn test_field_level_entry_does_not_match_uncovered_paths() {
        let ed = load_from_str(
            r#"
[[expected]]
method = "getblockchaininfo"
reason = "Only softforks is expected."
diff_paths = ["/softforks"]
"#,
        );
        // /chain is NOT covered — should not be expected
        let actual_paths = vec!["/softforks/0".to_string(), "/chain".to_string()];
        assert!(ed.is_expected("getblockchaininfo", &actual_paths).is_none());
    }

    #[test]
    fn test_wrong_method_does_not_match() {
        let ed = load_from_str(
            r#"
[[expected]]
method = "getnetworkinfo"
reason = "Some diff."
"#,
        );
        assert!(ed.is_expected("getblockchaininfo", &[]).is_none());
    }

    #[test]
    fn test_none_returns_empty() {
        let ed = ExpectedDiffs::none();
        assert!(ed.is_expected("anymethod", &[]).is_none());
    }

    // ── expected_missing ──────────────────────────────────────────────────────

    #[test]
    fn test_expected_missing_flag_is_recognized() {
        let ed = load_from_str(
            r#"
[[expected]]
method = "z_getaddressforaccount"
reason = "Not yet implemented in Zallet."
expected_missing = true
"#,
        );
        assert!(ed.is_expected_missing("z_getaddressforaccount").is_some());
        assert_eq!(
            ed.is_expected_missing("z_getaddressforaccount")
                .unwrap()
                .reason,
            "Not yet implemented in Zallet."
        );
    }

    #[test]
    fn test_expected_missing_false_is_not_returned() {
        let ed = load_from_str(
            r#"
[[expected]]
method = "getblockchaininfo"
reason = "Some diff."
diff_paths = ["/softforks"]
"#,
        );
        // expected_missing defaults to false — should NOT appear
        assert!(ed.is_expected_missing("getblockchaininfo").is_none());
    }

    #[test]
    fn test_expected_missing_wrong_method_does_not_match() {
        let ed = load_from_str(
            r#"
[[expected]]
method = "z_getaddressforaccount"
reason = "Not yet implemented."
expected_missing = true
"#,
        );
        assert!(ed.is_expected_missing("getblockchaininfo").is_none());
    }
}
