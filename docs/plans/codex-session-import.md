# Codex import implementation

Implements [project-session-import](../specs/project-session-import.md).

1. Filter Codex provenance before parsing and isolate project storage.
2. Persist content-addressed digest results and recovery splits before advancing.
3. Bound calls and prevent concurrent workers with an OS-backed workspace lock.
4. Carry optional sanitised failure details and checkpoint progress over the bus.
5. Validate offline restart, append, exclusion, failure and compatibility cases.
