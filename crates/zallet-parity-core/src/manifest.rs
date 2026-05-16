use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A manifest defining the suite of RPC methods to test.
#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub methods: Vec<MethodEntry>,
}

/// A single method entry in the parity manifest.
///
/// # Example TOML
/// ```toml
/// [[methods]]
/// name = "getblockchaininfo"
/// tags = ["blockchain", "core"]
/// ignore_paths = ["/blocks", "/verificationprogress"]
/// ```
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MethodEntry {
    pub name: String,
    pub params: Option<serde_json::Value>,
    /// JSON Pointer paths (RFC 6901) to remove from both responses
    /// before comparison. Useful for volatile or intentionally-divergent fields.
    #[serde(default)]
    pub ignore_paths: Vec<String>,
    /// Free-form labels used for `--tags` / `--exclude-tags` filtering.
    ///
    /// Examples: `["wallet", "shielded"]`, `["blockchain", "core"]`
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Manifest {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Manifest(format!("Failed to read manifest: {}", e)))?;

        toml::from_str(&content)
            .map_err(|e| Error::Manifest(format!("Failed to parse manifest: {}", e)))
    }

    /// Keeps only methods that have **at least one** of the given `tags`.
    ///
    /// If `tags` is empty the manifest is returned unchanged.
    pub fn filter_by_tags(self, tags: &[String]) -> Self {
        if tags.is_empty() {
            return self;
        }
        Self {
            methods: self
                .methods
                .into_iter()
                .filter(|m| m.tags.iter().any(|t| tags.contains(t)))
                .collect(),
        }
    }

    /// Removes methods that have **at least one** of the given `exclude_tags`.
    ///
    /// If `exclude_tags` is empty the manifest is returned unchanged.
    pub fn filter_exclude_tags(self, exclude_tags: &[String]) -> Self {
        if exclude_tags.is_empty() {
            return self;
        }
        Self {
            methods: self
                .methods
                .into_iter()
                .filter(|m| !m.tags.iter().any(|t| exclude_tags.contains(t)))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_manifest(toml: &str) -> Manifest {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "{}", toml).unwrap();
        Manifest::load(file.path()).unwrap()
    }

    #[test]
    fn test_load_manifest_with_ignore_paths() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
[[methods]]
name = "getblockchaininfo"
ignore_paths = ["/blocks", "/verificationprogress"]

[[methods]]
name = "getwalletinfo"
"#
        )
        .unwrap();

        let manifest = Manifest::load(file.path()).unwrap();
        assert_eq!(manifest.methods.len(), 2);

        let m0 = &manifest.methods[0];
        assert_eq!(m0.name, "getblockchaininfo");
        assert_eq!(m0.ignore_paths, vec!["/blocks", "/verificationprogress"]);

        let m1 = &manifest.methods[1];
        assert_eq!(m1.name, "getwalletinfo");
        assert!(m1.ignore_paths.is_empty()); // defaults to empty vec
    }

    #[test]
    fn test_load_manifest_no_ignore_paths_defaults_empty() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "[[methods]]\nname = \"getinfo\"").unwrap();

        let manifest = Manifest::load(file.path()).unwrap();
        assert!(manifest.methods[0].ignore_paths.is_empty());
    }

    // ── Tag filtering ─────────────────────────────────────────────────────────

    const TAG_MANIFEST: &str = r#"
[[methods]]
name = "getblockchaininfo"
tags = ["blockchain", "core"]

[[methods]]
name = "getbalance"
tags = ["wallet", "balance"]

[[methods]]
name = "z_gettotalbalance"
tags = ["wallet", "shielded"]

[[methods]]
name = "getmininginfo"
tags = ["mining"]
"#;

    #[test]
    fn test_filter_by_tags_returns_matching_methods() {
        let m = make_manifest(TAG_MANIFEST).filter_by_tags(&["wallet".to_string()]);
        assert_eq!(m.methods.len(), 2);
        assert!(
            m.methods
                .iter()
                .all(|e| e.tags.contains(&"wallet".to_string()))
        );
    }

    #[test]
    fn test_filter_by_tags_empty_tags_returns_all() {
        let m = make_manifest(TAG_MANIFEST).filter_by_tags(&[]);
        assert_eq!(m.methods.len(), 4);
    }

    #[test]
    fn test_filter_by_tags_multiple_tags_union() {
        let m = make_manifest(TAG_MANIFEST)
            .filter_by_tags(&["blockchain".to_string(), "mining".to_string()]);
        assert_eq!(m.methods.len(), 2);
        let names: Vec<_> = m.methods.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"getblockchaininfo"));
        assert!(names.contains(&"getmininginfo"));
    }

    #[test]
    fn test_filter_exclude_tags_removes_matching() {
        let m = make_manifest(TAG_MANIFEST).filter_exclude_tags(&["wallet".to_string()]);
        assert_eq!(m.methods.len(), 2);
        assert!(
            !m.methods
                .iter()
                .any(|e| e.tags.contains(&"wallet".to_string()))
        );
    }

    #[test]
    fn test_filter_exclude_tags_empty_returns_all() {
        let m = make_manifest(TAG_MANIFEST).filter_exclude_tags(&[]);
        assert_eq!(m.methods.len(), 4);
    }

    #[test]
    fn test_filter_include_then_exclude_shielded() {
        // Include all wallet methods, then exclude shielded ones.
        let m = make_manifest(TAG_MANIFEST)
            .filter_by_tags(&["wallet".to_string()])
            .filter_exclude_tags(&["shielded".to_string()]);
        assert_eq!(m.methods.len(), 1);
        assert_eq!(m.methods[0].name, "getbalance");
    }
}
