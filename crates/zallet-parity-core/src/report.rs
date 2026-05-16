use crate::differ::DiffEntry;
use crate::engine::ParityResult;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── Report types ──────────────────────────────────────────────────────────────

/// The final report structure for a parity run.
#[derive(Debug, Serialize, Deserialize)]
pub struct FinalReport {
    /// Schema version for forward-compatibility; currently `"1"`.
    pub schema_version: &'static str,
    /// ISO-8601 UTC timestamp of when this report was generated.
    pub generated_at: String,
    pub summary: RunSummary,
    /// Per-method results, ordered lexicographically by method name.
    pub details: BTreeMap<String, ParityResultReport>,
}

/// Aggregate counts for the run summary.
#[derive(Debug, Serialize, Deserialize)]
pub struct RunSummary {
    pub total: usize,
    pub matches: usize,
    pub diffs: usize,
    pub expected_diffs: usize,
    pub missing: usize,
    pub errors: usize,
}

/// The serialized form of a single method's parity result.
///
/// The `type` tag distinguishes the variant in JSON:
/// `"match"`, `"diff"`, `"expected_diff"`, `"missing"`, `"error"`
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParityResultReport {
    Match,
    /// Unexpected diff — represents a real compatibility gap.
    Diff {
        /// Number of leaf-level differences found.
        diff_count: usize,
        /// JSON Pointer paths where differences were found.
        diff_paths: Vec<String>,
    },
    /// Known/intentional diff — visible in the report but not a blocker.
    ExpectedDiff {
        diff_count: usize,
        diff_paths: Vec<String>,
        /// Human-readable explanation from the expected-diffs file.
        reason: String,
    },
    Missing {
        method: String,
    },
    Error {
        message: String,
    },
}

// ── FinalReport construction ──────────────────────────────────────────────────

impl FinalReport {
    /// Builds a `FinalReport` from a list of `(method_name, ParityResult)` pairs.
    pub fn new(results: Vec<(String, ParityResult)>) -> Self {
        let mut matches = 0usize;
        let mut diffs = 0usize;
        let mut expected_diffs = 0usize;
        let mut missing = 0usize;
        let mut errors = 0usize;
        let mut details = BTreeMap::new();

        for (method, res) in results {
            let report_res = classify_result(
                res,
                &mut matches,
                &mut diffs,
                &mut expected_diffs,
                &mut missing,
                &mut errors,
            );
            details.insert(method, report_res);
        }

        Self {
            schema_version: "1",
            generated_at: generated_at_now(),
            summary: RunSummary {
                total: details.len(),
                matches,
                diffs,
                expected_diffs,
                missing,
                errors,
            },
            details,
        }
    }

    /// Returns the raw `DiffEntry` objects for a given method (for verbose/debug output).
    pub fn with_diff_detail(
        results: Vec<(String, ParityResult)>,
    ) -> (Self, BTreeMap<String, Vec<DiffEntry>>) {
        let mut raw_diffs: BTreeMap<String, Vec<DiffEntry>> = BTreeMap::new();
        let mapped: Vec<(String, ParityResult)> = results
            .into_iter()
            .map(|(method, res)| {
                match &res {
                    ParityResult::Diff { diff_entries }
                    | ParityResult::ExpectedDiff { diff_entries, .. } => {
                        raw_diffs.insert(method.clone(), diff_entries.clone());
                    }
                    _ => {}
                }
                (method, res)
            })
            .collect();
        (Self::new(mapped), raw_diffs)
    }

    /// Renders the report as a Markdown document suitable for human review or PR comments.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str(&render_summary_section(&self.summary));
        md.push_str(&render_details_table(&self.details));
        md
    }
}

// ── Timestamp helper ─────────────────────────────────────────────────────────────

/// Returns the current UTC time as an ISO-8601 string.
///
/// Uses only `std` — avoids an external chrono/time dep.
fn generated_at_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // RFC 3339 / ISO-8601 — seconds precision is sufficient for report stamping.
    let (y, mo, d, h, mi, s) = unix_secs_to_datetime(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, mi, s)
}

/// Converts UNIX seconds to (year, month, day, hour, minute, second).
/// Implements a minimal Gregorian calendar calculation without external deps.
fn unix_secs_to_datetime(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    let mins = secs / 60;
    let mi = mins % 60;
    let hours = mins / 60;
    let h = hours % 24;
    let mut days = hours / 24;

    // Gregorian calendar epoch: 1 Jan 1970
    let mut y = 1970u64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        y += 1;
    }
    let months = [31, if is_leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 1u64;
    for m in months {
        if days < m {
            break;
        }
        days -= m;
        mo += 1;
    }
    (y, mo, days + 1, h, mi, s)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

// ── Construction helpers ──────────────────────────────────────────────────────

/// Converts a single `ParityResult` to its serializable `ParityResultReport` form,
/// incrementing the appropriate counter as a side effect.
fn classify_result(
    res: ParityResult,
    matches: &mut usize,
    diffs: &mut usize,
    expected_diffs: &mut usize,
    missing: &mut usize,
    errors: &mut usize,
) -> ParityResultReport {
    match res {
        ParityResult::Match => {
            *matches += 1;
            ParityResultReport::Match
        }
        ParityResult::Diff { diff_entries } => {
            *diffs += 1;
            let diff_paths = extract_paths(&diff_entries);
            ParityResultReport::Diff {
                diff_count: diff_paths.len(),
                diff_paths,
            }
        }
        ParityResult::ExpectedDiff {
            diff_entries,
            reason,
        } => {
            *expected_diffs += 1;
            let diff_paths = extract_paths(&diff_entries);
            ParityResultReport::ExpectedDiff {
                diff_count: diff_paths.len(),
                diff_paths,
                reason,
            }
        }
        ParityResult::Missing { method: m } => {
            *missing += 1;
            ParityResultReport::Missing { method: m }
        }
        ParityResult::Error(message) => {
            *errors += 1;
            ParityResultReport::Error { message }
        }
    }
}

/// Extracts the JSON Pointer path strings from a list of `DiffEntry` items.
fn extract_paths(entries: &[DiffEntry]) -> Vec<String> {
    entries.iter().map(|e| e.path.clone()).collect()
}

// ── Markdown rendering helpers ────────────────────────────────────────────────

/// Renders the summary block at the top of the Markdown report.
fn render_summary_section(s: &RunSummary) -> String {
    format!(
        "# Zallet Parity Report\n\n\
         - **Total Tests**: {total}\n\
         - **✅ Matches**: {matches}\n\
         - **❌ Diffs**: {diffs}\n\
         - **📋 Expected Diffs**: {expected_diffs}\n\
         - **🔍 Missing**: {missing}\n\
         - **⚠️ Errors**: {errors}\n\n",
        total = s.total,
        matches = s.matches,
        diffs = s.diffs,
        expected_diffs = s.expected_diffs,
        missing = s.missing,
        errors = s.errors,
    )
}

/// Renders the per-method detailed results as a sorted Markdown table.
fn render_details_table(details: &BTreeMap<String, ParityResultReport>) -> String {
    let mut md = String::from("## Detailed Results\n\n");
    md.push_str("| Method | Status | Details |\n");
    md.push_str("| :--- | :--- | :--- |\n");

    // BTreeMap iterates in sorted order — no manual sort needed.
    for (method, res) in details {
        let (status, notes) = format_result_row(res);
        md.push_str(&format!(
            "| `{}` | {} | {} |\n",
            method,
            status,
            notes.replace('\n', "<br>")
        ));
    }

    md
}

/// Returns the (status emoji+label, detail notes) pair for one result row.
fn format_result_row(res: &ParityResultReport) -> (&'static str, String) {
    match res {
        ParityResultReport::Match => ("✅ Match", String::new()),
        ParityResultReport::Diff {
            diff_count,
            diff_paths,
        } => (
            "❌ Diff",
            format!(
                "{} field(s) differ: `{}`",
                diff_count,
                diff_paths.join(", ")
            ),
        ),
        ParityResultReport::ExpectedDiff {
            diff_count,
            diff_paths,
            reason,
        } => (
            "📋 Expected Diff",
            format!(
                "{} field(s): `{}` — _{}_ ",
                diff_count,
                diff_paths.join(", "),
                reason
            ),
        ),
        ParityResultReport::Missing { method } => (
            "🔍 Missing",
            format!("Method `{}` not found on one endpoint", method),
        ),
        ParityResultReport::Error { message } => ("⚠️ Error", message.clone()),
    }
}
