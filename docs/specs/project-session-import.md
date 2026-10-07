# Codex project-scoped session import

The driver reads optional `persona/import-scope.json` under its workspace:
`{"project_root":"/absolute/path/to/project"}`. Invalid configured scope fails
closed; no file means the existing general Codex/Claude import. No path is
accepted through the coding-session RPC contract.

A configured project imports Codex only. It accepts existing absolute recorded
working directories within the canonical project, including children, and
rejects missing provenance and symlink escapes. Status and ingestion use the
same predicate. Claude's general importer is unchanged and is not invoked by
a Codex project pass. Project cursors, window checkpoints and facet trees live
under a hash of the canonical project path. Publishing the project persona
preserves the prior general persona; unrelated document memory is untouched.

Each pass selects at most five sessions and spends at most five digest calls.
Scoped tree reduction shares that ceiling, using the existing deterministic
fallback when it is spent. Successful digest pieces, including empty results,
are stored atomically before the next request. Their keys include source,
provider and content fingerprints; recovery split decisions also persist.
Changed pieces are re-digested. Failed pieces remain retryable and never commit
a completed-file cursor. General Claude recovery behavior is unchanged.

The OS-backed workspace lock prevents overlapping import workers, including
when the UI or host RPC stops waiting. Restart releases the lock and resumes
persisted windows. A scan is bounded by metadata/file/byte limits; counts are
lower bounds when truncated. Failures contain stable codes and opaque session
identifiers, never transcript excerpts or raw provider messages. New report
fields default when reading older drivers. No provider or consent setting is
changed by ingestion.
