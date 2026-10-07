//! [`MemoryCodingSessions`] — distilling the user's coding-agent transcripts.
//!
//! A driver advertising
//! [`Capability::CodingSessions`](crate::capabilities::Capability::CodingSessions)
//! knows where a coding agent leaves its session transcripts, can say how much
//! is there without ingesting any of it, and can run the pass that turns those
//! transcripts into observations about the user.
//!
//! # Why not the source-sync family
//!
//! Both fetch and report, and that is the whole of the resemblance.
//! [`MemorySourceSync`](super::MemorySourceSync) walks a *remote* connection
//! the user authorised, is billed per provider action, and resumes from a
//! provider cursor. This walks *local* files the user's own tools wrote, is
//! billed per inference window, and resumes from a per-file state store. The
//! two fail independently, which is the test that decides a family: a driver
//! running server-side has no `~/.claude` to read, and a driver fronting a
//! local vault may have no authorised connection to walk. Advertising them
//! together puts a dead control in front of whichever half is absent.
//!
//! # No path crosses this contract
//!
//! Not one member takes a directory. Which agents are supported, where each
//! keeps its sessions, and how the environment overrides those locations are
//! resolved driver-side. A caller passing roots would be choosing which files
//! the driver opens — the shape a source gate exists to prevent — and would
//! freeze the supported-agent list into the contract, where adding an agent
//! becomes a version bump instead of a driver release.
//!
//! # Both members are bounded, and say when the bound bit
//!
//! A status scan caps the files it opens and the bytes it reads; an ingest
//! caps the sessions it processes. Neither is a promise to finish: a large
//! history drains across repeated calls, and
//! [`CodingSessionSource::scan_truncated`] and
//! [`CodingSessionIngestReport::budget_hit`] are how a caller knows to ask
//! again rather than to report a total it has not seen.

use async_trait::async_trait;

use crate::error::MemoryError;

// The value types this family exchanges — defined in `tinymemory-bus` because
// they cross the module boundary, re-exported here so the two trees stay the
// same shape and the types stay the same types.
pub use tinymemory_bus::provider::sessions::{
    CodingSessionFailure, CodingSessionIngestReport, CodingSessionIngestRequest,
    CodingSessionSource,
};

/// Reading and distilling local coding-agent session transcripts.
#[async_trait]
pub trait MemoryCodingSessions: Send + Sync {
    /// What each supported agent's session store holds right now.
    ///
    /// One row per agent the driver knows about, present or not — an absent
    /// agent is a row with [`CodingSessionSource::available`] `false`, not a
    /// missing row, because a caller rendering a picker needs to show what it
    /// could offer as well as what it can.
    ///
    /// Bounded by the driver's own scan caps rather than by an argument: the
    /// caps exist to keep a status call from reading a multi-gigabyte history,
    /// and a caller able to raise them could turn a status probe into one.
    ///
    /// # Errors
    ///
    /// Backend failures only. A file that cannot be read is counted in
    /// [`CodingSessionSource::invalid_files`] rather than raised — one
    /// half-written transcript from a session that is still running must not
    /// fail a scan of four hundred.
    async fn coding_session_status(&self) -> Result<Vec<CodingSessionSource>, MemoryError>;

    /// Distil coding sessions into observations, and report what the pass did.
    ///
    /// # This costs inference, and the caller cannot bound the time
    ///
    /// Each session is one or more sequential model calls, so the wall-clock
    /// cost scales with [`CodingSessionIngestRequest::max_sessions`] and with
    /// how long the individual transcripts are. The driver clamps the request
    /// to its own ceiling; a caller that needs a deadline enforces it on its
    /// own side, because a driver that abandoned a run mid-session would leave
    /// a state store that disagrees with what was written.
    ///
    /// # Errors
    ///
    /// [`MemoryError::Unsupported`] from a driver with no summarisation
    /// provider resolvable — deliberately not an empty report, which would
    /// tell a user their history was imported and found nothing in it.
    ///
    /// [`MemoryError::BudgetExceeded`] when an inference budget stops the run
    /// before it processed anything. A run that stopped on its *session*
    /// budget after doing work is an `Ok` with
    /// [`CodingSessionIngestReport::budget_hit`] set, because that is progress
    /// the caller should keep and continue from.
    ///
    /// Otherwise backend failures. Individual failed sessions are counted in
    /// [`CodingSessionIngestReport::sessions_failed`], for the same reason the
    /// status scan counts unreadable files.
    async fn ingest_coding_sessions(
        &self,
        request: CodingSessionIngestRequest,
    ) -> Result<CodingSessionIngestReport, MemoryError>;
}
