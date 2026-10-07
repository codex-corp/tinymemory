//! The coding-sessions family: transcripts of the user's agent sessions, read
//! and distilled by the driver.
//!
//! A driver advertising
//! [`Capability::CodingSessions`](crate::capabilities::Capability::CodingSessions)
//! knows where a coding agent leaves its session transcripts, can say how much
//! is there without reading any of it into memory, and can run the distillation
//! pass that turns those transcripts into observations about the user.
//!
//! # Why this is not the source-sync family
//!
//! Both are "go and fetch, then tell me what you got", and that is where the
//! resemblance stops. A source sync walks a *remote* connection the user
//! authorised, is billed per provider action, and resumes from a cursor. This
//! walks *local* files the user's own tools wrote, is billed per inference
//! window, and resumes from a per-file state store. A driver that can do one
//! and not the other is the ordinary case rather than the exotic one — a
//! server-side driver has no `~/.claude` to read, and a driver fronting a
//! local vault has no Composio connection — so they negotiate separately.
//!
//! # Where the transcripts are is the driver's business
//!
//! No member here takes a path. Which agents are supported, where each keeps
//! its sessions, and how the environment overrides those locations are all
//! resolved driver-side. A caller passing roots would be choosing which files
//! the driver reads, which is exactly the shape a source gate exists to
//! prevent — and it would freeze the supported-agent list into the contract,
//! where adding one becomes a version bump.

use serde::{Deserialize, Serialize};

/// What one coding agent's session store holds, without ingesting any of it.
///
/// The answer behind a "there are 412 sessions to import" prompt. Reading it is
/// bounded work: the driver caps how many files it opens and how many bytes it
/// reads, and says so in [`Self::scan_truncated`] rather than taking however
/// long a large history needs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingSessionSource {
    /// Which agent this row is for, as the driver names it (`claude_code`,
    /// `codex`, …).
    ///
    /// A wire string rather than an enum for the same reason a source kind is
    /// one on the sink family: the supported set is the driver's, and it grows
    /// as agents are added without a contract change.
    pub kind: String,
    /// Whether the driver found this agent's session store at all.
    ///
    /// `false` with zero counts is "this agent is not installed"; `true` with
    /// zero counts is "installed, nothing recorded". A caller prompts for the
    /// second and stays quiet about the first.
    pub available: bool,
    /// Session files the scan saw.
    pub session_files: usize,
    /// Evidence units those files parse into — the unit the ingest budget is
    /// spent in, so this is what a caller sizes an import against.
    pub evidence_units: usize,
    /// Files the scan could not read or parse.
    ///
    /// Not an error: a half-written transcript from a session that is still
    /// running is normal, and the count is what tells a caller its total is a
    /// floor.
    pub invalid_files: usize,
    /// Whether the scan stopped at one of its own caps rather than at the end.
    ///
    /// Every count above is then a floor. The caller shows "412+" rather than
    /// "412", and the difference matters on the one screen where the number is
    /// a promise about how long an import will take.
    pub scan_truncated: bool,
    /// Driver-owned Codex project restriction, when configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_scope: Option<String>,
    /// Files excluded because their project provenance did not match.
    #[serde(default)]
    pub sessions_excluded: usize,
}

/// Sanitised diagnostic for a session that remains retryable.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingSessionFailure {
    /// Stable machine-readable cause; no raw provider message.
    pub code: String,
    /// Opaque correlation identifier, not a local file path.
    pub session_id: String,
    /// Safe explanation and remediation.
    pub summary: String,
}

/// A request to distil coding sessions into observations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingSessionIngestRequest {
    /// Re-read sessions the driver has already processed.
    ///
    /// `false` — the default — processes only what is new since the last run.
    /// `true` is the "import my history" pass, and costs an inference window
    /// per session all over again.
    #[serde(default)]
    pub backfill: bool,
    /// How many sessions this run may process.
    ///
    /// The driver clamps it to its own floor and ceiling: a caller cannot raise
    /// the limit by asking for more, the same rule
    /// [`crate::provider::chunks::ChunkQuery::limit`] carries. Bounded because
    /// each session is one or more sequential LLM calls, so an unbounded run is
    /// an unbounded bill and an unbounded wall-clock wait.
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,
}

/// The `max_sessions` an older caller's payload means.
///
/// A caller that omits the field is asking for the driver's ordinary batch, not
/// for none and not for all of history — so the default is a real batch size
/// rather than `0` (which would silently do nothing) or `usize::MAX` (which
/// would silently do everything). The driver clamps it either way.
fn default_max_sessions() -> usize {
    100
}

impl Default for CodingSessionIngestRequest {
    /// Incremental, at the default batch size — the shape a scheduler asks for.
    fn default() -> Self {
        Self {
            backfill: false,
            max_sessions: default_max_sessions(),
        }
    }
}

/// What one coding-session ingest run read, distilled and skipped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingSessionIngestReport {
    /// Which pass ran, in the driver's own words (`incremental`, `backfill`).
    ///
    /// Echoed rather than assumed: a driver that has never run before may
    /// upgrade an incremental request to a full pass, and a caller reporting
    /// "up to date" over that would be describing the wrong run.
    pub mode: String,
    /// Session files the run looked at.
    pub files_seen: usize,
    /// Sessions it distilled.
    pub sessions_processed: usize,
    /// Sessions it skipped because their state said they were already done.
    pub sessions_skipped: usize,
    /// Sessions it attempted and failed.
    ///
    /// Counted rather than raised: one unreadable transcript must not abandon
    /// the other four hundred, and a caller decides from the ratio whether
    /// anything is actually wrong.
    pub sessions_failed: usize,
    /// Evidence units the run consumed.
    pub evidence_units: usize,
    /// Observations it wrote.
    pub observations: usize,
    /// Whether the run stopped on its budget rather than on running out of
    /// sessions.
    ///
    /// `true` means calling again makes more progress, which is how a caller
    /// drains a large history across several passes instead of one call that
    /// cannot finish.
    pub budget_hit: bool,
    /// Where the driver wrote the distilled pack, when it writes one and when
    /// the location is meaningful to the caller.
    ///
    /// `None` from a driver that keeps the result in its own storage. A caller
    /// must not require this to be `Some` — it is a convenience for a local
    /// driver, not a promise that the output is a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack_path: Option<String>,
    /// Newly persisted digest pieces or recovery splits; progress before completion.
    #[serde(default)]
    pub checkpoints_advanced: usize,
    /// Safe, bounded failures retained for retry.
    #[serde(default)]
    pub failures: Vec<CodingSessionFailure>,
    /// Codex transcripts excluded by project provenance.
    #[serde(default)]
    pub sessions_excluded: usize,
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
