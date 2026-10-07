//! Tests for the coding-session value types.
//!
//! The load-bearing part is [`CodingSessionIngestRequest`]'s default: the field
//! is `#[serde(default)]`, so what an older caller's payload *means* is decided
//! here rather than at whichever call site forgot to set it.

// A failed assertion in a test is a panic either way; `unwrap`/`expect` here say
// what the invariant was. Same allowance the crate's other test modules take.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::*;

#[test]
fn an_omitted_max_sessions_means_a_batch_not_none_and_not_everything() {
    // `0` would silently ingest nothing and report success; `usize::MAX` would
    // silently start an unbounded, billable run. Both are worse than a batch.
    let request: CodingSessionIngestRequest =
        serde_json::from_value(serde_json::json!({})).expect("decode an empty request");
    assert!(!request.backfill);
    assert_eq!(request.max_sessions, 100);
    assert_eq!(request, CodingSessionIngestRequest::default());
}

#[test]
fn backfill_defaults_to_the_cheap_pass() {
    // An absent flag must not mean "re-read all of history": incremental is the
    // pass a scheduler can run unattended.
    let request: CodingSessionIngestRequest =
        serde_json::from_value(serde_json::json!({ "max_sessions": 5 }))
            .expect("decode a partial request");
    assert!(!request.backfill);
    assert_eq!(request.max_sessions, 5);
}

#[test]
fn an_absent_agent_is_distinguishable_from_an_empty_one() {
    // The pair `(available, session_files)` carries two different prompts, and
    // a caller that read only the count would nag about an agent that is not
    // installed.
    let absent = CodingSessionSource {
        kind: "codex".to_string(),
        available: false,
        ..CodingSessionSource::default()
    };
    let empty = CodingSessionSource {
        kind: "codex".to_string(),
        available: true,
        ..CodingSessionSource::default()
    };
    assert_ne!(absent, empty);
    assert_eq!(absent.session_files, empty.session_files);
}

#[test]
fn an_ingest_report_without_a_pack_path_omits_it() {
    let report = CodingSessionIngestReport {
        mode: "incremental".to_string(),
        ..CodingSessionIngestReport::default()
    };
    let encoded = serde_json::to_value(&report).expect("serialize report");
    assert!(encoded.get("pack_path").is_none());
    assert_eq!(encoded["budget_hit"], serde_json::json!(false));
}

#[test]
fn older_reports_and_statuses_default_optional_import_fields() {
    let source: CodingSessionSource = serde_json::from_value(serde_json::json!({
        "kind":"codex", "available":true, "session_files":1, "evidence_units":2,
        "invalid_files":0, "scan_truncated":false
    }))
    .expect("legacy source");
    assert_eq!(source.project_scope, None);
    assert_eq!(source.sessions_excluded, 0);
    let report: CodingSessionIngestReport = serde_json::from_value(serde_json::json!({
        "mode":"incremental", "files_seen":1, "sessions_processed":1,
        "sessions_skipped":0, "sessions_failed":0, "evidence_units":2,
        "observations":1, "budget_hit":false
    }))
    .expect("legacy report");
    assert_eq!(report.checkpoints_advanced, 0);
    assert!(report.failures.is_empty());
}
