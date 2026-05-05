# CASS Test Fixtures

These fixtures replace inline mock session builders in tests.

- `sessions/session-001.jsonl`: focused Rust debugging transcript
- `sessions/session-002.jsonl`: small refactor transcript
- `sessions/session-003.jsonl`: flaky test investigation transcript
- `extractions/debugging-skill.json`: extracted skill metadata tied to the sessions

Constraints:

- Session fixtures stay under 1KB each
- Content is intentionally small but shaped like the JSONL event stream tests expect
- Extraction data stays deterministic so copy-based tests can assert exact fields
