//! Host orchestration for TinyCortex coding-session persona ingestion.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tinycortex::memory::persona::readers::{claude_code, codex, RawSession};
use tinycortex::memory::persona::state::FileStateStore;
use tinycortex::memory::persona::{PersonaConfig, Pipeline, RunMode};
use walkdir::WalkDir;

use crate::Config;

const DEFAULT_MAX_SESSIONS: usize = 100;
const MAX_MAX_SESSIONS: usize = 1_000;
const MAX_STATUS_SESSION_FILES: usize = 1_000;
const MAX_STATUS_SESSION_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_STATUS_TOTAL_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodingSessionSourceStatus {
    pub kind: String,
    pub available: bool,
    pub session_files: usize,
    pub evidence_units: usize,
    pub invalid_files: usize,
    pub scan_truncated: bool,
    pub project_scope: Option<String>,
    pub sessions_excluded: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodingSessionIngestRequest {
    #[serde(default)]
    pub backfill: bool,
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,
}

fn default_max_sessions() -> usize {
    DEFAULT_MAX_SESSIONS
}

#[derive(Debug, Clone, Serialize)]
pub struct CodingSessionIngestResponse {
    pub mode: String,
    pub files_seen: usize,
    pub sessions_processed: usize,
    pub sessions_skipped: usize,
    pub sessions_failed: usize,
    pub evidence_units: usize,
    pub observations: usize,
    pub budget_hit: bool,
    pub pack_path: Option<String>,
    pub checkpoints_advanced: usize,
    pub failures: Vec<tinycortex::memory::persona::pipeline::SessionFailure>,
    pub sessions_excluded: usize,
}

fn roots_from_environment() -> (PathBuf, PathBuf) {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let claude_home = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    (claude_home.join("projects"), codex_home.join("sessions"))
}

fn source_status(
    kind: &str,
    root: &Path,
    max_files: usize,
    discover: impl Fn(&Path, usize) -> (Vec<PathBuf>, bool),
    read: impl Fn(&Path) -> anyhow::Result<RawSession>,
) -> CodingSessionSourceStatus {
    source_status_scoped(kind, root, max_files, discover, read, None)
}

fn source_status_scoped(
    kind: &str,
    root: &Path,
    max_files: usize,
    discover: impl Fn(&Path, usize) -> (Vec<PathBuf>, bool),
    read: impl Fn(&Path) -> anyhow::Result<RawSession>,
    project: Option<&Path>,
) -> CodingSessionSourceStatus {
    let (files, mut scan_truncated) = discover(root, max_files);
    let mut matched_files = 0;
    let mut excluded = 0;
    if scan_truncated {
        tracing::debug!(
            source = kind,
            max_files,
            "[memory_persona] coding session status scan capped"
        );
    }
    let mut evidence_units = 0;
    let mut invalid_files = 0;
    let mut bytes_scheduled = 0_u64;
    for path in &files {
        if let Some(project) = project {
            let scope = tinycortex::memory::persona::scope::transcript_scope(path, kind)
                .ok()
                .flatten();
            if !tinycortex::memory::persona::scope::matches_project(scope.as_deref(), project) {
                excluded += 1;
                continue;
            }
            matched_files += 1;
        }
        if let Ok(metadata) = path.metadata() {
            let file_bytes = metadata.len();
            if file_bytes > MAX_STATUS_SESSION_FILE_BYTES
                || bytes_scheduled.saturating_add(file_bytes) > MAX_STATUS_TOTAL_BYTES
            {
                scan_truncated = true;
                tracing::debug!(
                    source = kind,
                    file_bytes,
                    bytes_scheduled,
                    max_file_bytes = MAX_STATUS_SESSION_FILE_BYTES,
                    max_total_bytes = MAX_STATUS_TOTAL_BYTES,
                    reason = "status-byte-budget",
                    "[memory_persona] skipped coding session during bounded status scan"
                );
                continue;
            }
            bytes_scheduled += file_bytes;
        }
        match read(path) {
            Ok(session) => evidence_units += session.evidence.len(),
            Err(_error) => {
                invalid_files += 1;
                tracing::debug!(
                    source = kind,
                    reason = "read-or-parse-failed",
                    "[memory_persona] skipped unreadable coding session"
                );
            }
        }
    }
    CodingSessionSourceStatus {
        kind: kind.to_string(),
        available: root.is_dir(),
        session_files: if project.is_some() {
            matched_files
        } else {
            files.len()
        },
        evidence_units,
        invalid_files,
        scan_truncated,
        project_scope: project.map(|p| p.display().to_string()),
        sessions_excluded: excluded,
    }
}

fn discover_session_files(
    root: &Path,
    max_files: usize,
    is_candidate: impl Fn(&Path) -> bool,
) -> (Vec<PathBuf>, bool) {
    let mut files = Vec::with_capacity(max_files.min(64));
    // Keep traversal unsorted: `sort_by_file_name` buffers and sorts every
    // directory before yielding its first entry, which defeats `max_files`
    // for users with very large Codex day or Claude project directories.
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if !is_candidate(path) {
            continue;
        }
        if files.len() == max_files {
            return (files, true);
        }
        files.push(path.to_path_buf());
    }
    (files, false)
}

fn discover_claude_sessions(root: &Path, max_files: usize) -> (Vec<PathBuf>, bool) {
    discover_session_files(root, max_files, |path| {
        path.extension()
            .is_some_and(|extension| extension == "jsonl")
    })
}

fn discover_codex_sessions(root: &Path, max_files: usize) -> (Vec<PathBuf>, bool) {
    discover_session_files(root, max_files, |path| {
        path.extension()
            .is_some_and(|extension| extension == "jsonl")
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rollout-"))
    })
}

pub fn coding_session_status_for_roots(
    claude_root: &Path,
    codex_root: &Path,
) -> Vec<CodingSessionSourceStatus> {
    tracing::debug!("[memory_persona] coding session scan: entry");
    let statuses = vec![
        source_status(
            "claude_code",
            claude_root,
            MAX_STATUS_SESSION_FILES,
            discover_claude_sessions,
            claude_code::read_session,
        ),
        source_status(
            "codex",
            codex_root,
            MAX_STATUS_SESSION_FILES,
            discover_codex_sessions,
            codex::read_session,
        ),
    ];
    tracing::debug!(
        files = statuses
            .iter()
            .map(|status| status.session_files)
            .sum::<usize>(),
        evidence = statuses
            .iter()
            .map(|status| status.evidence_units)
            .sum::<usize>(),
        invalid = statuses
            .iter()
            .map(|status| status.invalid_files)
            .sum::<usize>(),
        "[memory_persona] coding session scan: exit"
    );
    statuses
}

pub fn coding_session_status() -> Vec<CodingSessionSourceStatus> {
    let (claude_root, codex_root) = roots_from_environment();
    coding_session_status_for_roots(&claude_root, &codex_root)
}

/// Read a persisted project restriction; invalid configuration fails closed.
fn transcript_project(config: &Config) -> anyhow::Result<Option<PathBuf>> {
    let file = config.workspace_dir().join("persona/import-scope.json");
    if !file.exists() {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(file)?)?;
    let root = value
        .get("project_root")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("project_root is required in persona/import-scope.json"))?;
    let path = PathBuf::from(root);
    anyhow::ensure!(path.is_absolute(), "project_root must be absolute");
    let path = path.canonicalize()?;
    anyhow::ensure!(path.is_dir(), "project_root must be a directory");
    Ok(Some(path))
}

/// Count only matching transcripts when the user has restricted the import.
pub fn coding_session_status_with_config(
    config: &Config,
) -> anyhow::Result<Vec<CodingSessionSourceStatus>> {
    let Some(project) = transcript_project(config)? else {
        return Ok(coding_session_status());
    };
    let (claude, codex) = roots_from_environment();
    Ok(vec![
        source_status_scoped(
            "claude_code",
            &claude,
            MAX_STATUS_SESSION_FILES,
            discover_claude_sessions,
            claude_code::read_session,
            None,
        ),
        source_status_scoped(
            "codex",
            &codex,
            MAX_STATUS_SESSION_FILES,
            discover_codex_sessions,
            codex::read_session,
            Some(&project),
        ),
    ])
}

pub async fn ingest_coding_sessions(
    config: &Config,
    request: CodingSessionIngestRequest,
) -> anyhow::Result<CodingSessionIngestResponse> {
    // A process-independent lock survives caller timeouts while the blocking
    // worker continues, and the OS releases it on crash. Keep the inode.
    let lock_dir = config.workspace_dir().join("persona");
    std::fs::create_dir_all(&lock_dir)?;
    let _lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_dir.join("coding-import.lock"))?;
    _lock.try_lock().map_err(|_| {
        anyhow::anyhow!("import_in_progress: an import is already running; wait for it to finish")
    })?;
    let (claude_root, codex_root) = roots_from_environment();
    let project = transcript_project(config)?;
    let ceiling = if project.is_some() {
        5
    } else {
        MAX_MAX_SESSIONS
    };
    let max_sessions = request.max_sessions.clamp(1, ceiling);
    let mode = if request.backfill {
        RunMode::Backfill
    } else {
        RunMode::Incremental
    };
    tracing::info!(
        mode = if request.backfill {
            "backfill"
        } else {
            "incremental"
        },
        max_sessions,
        "[memory_persona] coding session ingestion: entry"
    );

    let workspace = if let Some(root) = &project {
        let id: String = Sha256::digest(root.to_string_lossy().as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        config.workspace_dir().join("persona/projects").join(id)
    } else {
        config.workspace_dir().clone()
    };
    let mut memory_config = super::memory_config_from(config, workspace.clone());
    if project.is_some() {
        memory_config.content_root = None;
    }

    let mut persona = PersonaConfig::with_home(
        dirs::home_dir()
            .as_deref()
            .unwrap_or_else(|| Path::new(".")),
        "OpenHuman user",
    );
    persona.claude_code_root = if project.is_some() {
        None
    } else {
        Some(claude_root)
    };
    persona.codex_root = Some(codex_root);
    persona.codex_project_root = project.clone();
    if project.is_some() {
        persona.digest_concurrency = 1;
    }
    // This product surface is deliberately scoped to coding-session history.
    // Repository history and instruction files can be wired separately with
    // their own disclosure and cost controls.
    persona.project_roots.clear();
    persona.global_instruction_files.clear();
    persona.author_emails.clear();
    persona.run_budget.max_sessions = max_sessions;
    // Successful Codex pieces persist across passes, so long sessions advance
    // without opening an unbounded provider-call budget.
    persona.run_budget.max_llm_calls = if project.is_some() {
        5
    } else {
        max_sessions as u32
    };

    let (available, _) = crate::chat_host::summarizer_available(config);
    anyhow::ensure!(available,
        "summarization_unavailable: enable an authorised summarisation provider before importing sessions");
    let provider = super::build_chat_provider(config).inspect_err(|error| {
        tracing::error!(
            error = %error,
            "[memory_persona] coding session ingestion: build_chat_provider failed"
        );
    })?;
    let summariser = super::HostSummariser::new(config.to_arc());
    let store = FileStateStore::open_in_workspace(&workspace).inspect_err(|error| {
        tracing::error!(
            error = %error,
            "[memory_persona] coding session ingestion: open state store failed"
        );
    })?;
    let report = Pipeline {
        config: &memory_config,
        persona: &persona,
        provider: provider.as_ref(),
        summariser: &summariser,
        store: &store,
    }
    .run(mode)
    .await
    .inspect_err(|error| {
        tracing::error!(
            error = %error,
            "[memory_persona] coding session ingestion: pipeline run failed"
        );
    })?;

    if project.is_some() {
        // Publish only the project pack into the active persona location. The
        // project cursors and facet trees stay isolated from general history.
        if let Some(pack) = &report.pack_path {
            let active = config.workspace_dir().join("persona/PERSONA.md");
            let previous = config
                .workspace_dir()
                .join("persona/PERSONA.before-project-scope.md");
            if active.exists() && !previous.exists() {
                std::fs::copy(&active, previous)?;
            }
            let temporary = active.with_extension("md.project.tmp");
            std::fs::copy(pack, &temporary)?;
            std::fs::rename(temporary, active)?;
        }
    }

    tracing::info!(
        files_seen = report.files_seen,
        sessions_processed = report.sessions_processed,
        sessions_failed = report.sessions_failed,
        evidence_units = report.evidence_units,
        observations = report.observations,
        budget_hit = report.budget_hit,
        "[memory_persona] coding session ingestion: exit"
    );
    Ok(CodingSessionIngestResponse {
        mode: report.mode,
        files_seen: report.files_seen,
        sessions_processed: report.sessions_processed,
        sessions_skipped: report.sessions_skipped,
        sessions_failed: report.sessions_failed,
        evidence_units: report.evidence_units,
        observations: report.observations,
        budget_hit: report.budget_hit,
        pack_path: report.pack_path,
        checkpoints_advanced: report.checkpoints_advanced,
        failures: report.failures,
        sessions_excluded: report.sessions_excluded,
    })
}

#[cfg(test)]
#[path = "persona_tests.rs"]
mod tests;
