# meta_skill Token-Minimal Agent Workflow Plan — Round 4

**Goal:** Maximize agent usefulness while minimizing prompt tokens.

The intended product UX is:

1. User runs `ms init` in any repo.
2. `ms init` auto-discovers skills from installed agent providers such as `.claude/skills`, `.agents/skills`, `.codex/skills`, and similar locations.
3. `ms` snapshots those skills into `.ms` so they are usable even if the original provider folders are later deleted.
4. User pastes a very short `ms` blurb into `AGENTS.md`.
5. From that point on, the AI handles routing, loading, and reloading skills on demand.

The core token goal is not “inject all skills into context”.
The core token goal is: **`ms` knows the whole registry; the agent sees only the task-relevant slice.**

---

## Product Decision

`ms` must be optimized for **one-time setup by the human, ongoing autonomy by the agent**.

That means:

- `ms init` is the only required onboarding command for normal users.
- Provider skill directories are **import sources**, not long-term runtime dependencies.
- `AGENTS.md` is the only required always-on prompt surface.
- `ms route "<task>"` becomes the primary AI entry point.
- `ms list`, `ms search`, `ms show`, and `ms load` remain available, but they are supporting tools rather than the first step.

---

## Before vs After

### Before

1. User runs `ms init`.
2. User manually configures `skill_paths`.
3. User runs `ms index`.
4. Agent must guess whether to use `search`, `list`, `suggest`, or `load`.
5. Agent often over-loads skill content because there is no section routing.
6. If provider skill directories disappear, future indexing/loading may break.

### After

1. User runs `ms init`.
2. `ms init` discovers provider skill directories automatically.
3. `ms init` snapshots all discovered skills into `.ms/archive`.
4. `ms init` builds the runtime registry, search index, and routing metadata from the snapshot.
5. User may delete original provider skill directories.
6. User adds a very short `ms` blurb to `AGENTS.md`.
7. Agent starts each non-trivial task with `ms route "<task>" -O json`.
8. `ms route` returns only the top candidate skills, why they match, and the exact minimal `ms load` command to run.
9. Agent loads only the suggested section or level, then rehydrates deeper only if needed.

---

## Verified Codebase State

| Claim | Verdict | Notes |
|-------|---------|-------|
| `ms init` exists | ✅ | [src/cli/commands/init.rs](/data/projects/meta_skill/src/cli/commands/init.rs:1) bootstraps `.ms`, opens DB/archive/index, and performs provider discovery/import |
| `ms init` auto-imports provider skills | ✅ | [`init_human()`](/data/projects/meta_skill/src/cli/commands/init.rs:166) and [`init_robot()`](/data/projects/meta_skill/src/cli/commands/init.rs:230) both call `ProviderDiscovery::new()`, `discover()`, and `import_discovered_skills(...)` |
| `ms index` exists | ✅ | [src/cli/commands/index.rs](/data/projects/meta_skill/src/cli/commands/index.rs:1) indexes configured external paths |
| Runtime archive exists | ✅ | [src/storage/git.rs](/data/projects/meta_skill/src/storage/git.rs:249) can persist skill specs in `.ms/archive` |
| Resolver can load from archive | ✅ | [src/core/resolution.rs](/data/projects/meta_skill/src/core/resolution.rs:25) has `GitSkillRepository` backed by archive |
| Archive currently captures full skill assets bundle | ❌ Partial | `write_skill()` writes spec/markdown metadata, but this plan must also preserve scripts/references for source-folder deletion safety |
| `AGENTS.md` blurb already exists in docs | ✅ | [README.md](/data/projects/meta_skill/README.md:691) has a prepared blurb, but it is too long and too human-centric for the final UX |
| `SKILL.md` generator exists | ✅ | [src/skill_md/mod.rs](/data/projects/meta_skill/src/skill_md/mod.rs:1) useful, but no longer the main always-on surface |
| `ms route` command exists | ✅ | [src/cli/commands/route.rs](/data/projects/meta_skill/src/cli/commands/route.rs:1) implements route schema, thresholding, candidates, and fallback |
| `--section` loading exists | ✅ | [src/cli/commands/load.rs](/data/projects/meta_skill/src/cli/commands/load.rs:1) supports `DisclosureLevel::Section` and slug-based section loading |

---

## Hard Constraints

1. The user workflow must start and end with `ms init` plus a short `AGENTS.md` snippet.
2. The agent must not need a giant startup manifest that enumerates every skill inline.
3. The authoritative runtime source after import is `.ms`, not `.claude/skills` or other provider folders.
4. The agent’s first tool call for non-trivial work should be `ms route`, not free-form `ms search`.
5. All routing outputs must be compact and machine-oriented.
6. The plan must preserve the user’s freedom to delete provider skill folders after initialization.

---

## Research-Backed Upgrades

The following additions are supported by current official docs and should shape the implementation.

### 1. Use four-tier lazy disclosure, not one big manifest

Anthropic’s current skill guide describes a three-level disclosure model:

- frontmatter always available
- `SKILL.md` body loaded when relevant
- bundled files discovered only as needed

Claude Code’s MCP docs add an analogous pattern for tools: only tool names load initially, and full tool definitions are deferred until needed. `ms` should combine those ideas into a runtime design like this:

1. **Tier A: Route index**
   Internal only. Tiny routing metadata used by `ms route`.
2. **Tier B: Candidate projection**
   Returned by `ms route` for top matches only.
3. **Tier C: Skill body**
   Loaded via `ms load --section` or `--level overview`.
4. **Tier D: References and scripts**
   Loaded only on explicit follow-up.

This is stricter than the current plan wording and better matches the stated token goal.

### 2. Make trigger phrases first-class metadata

Anthropic’s skill guidance emphasizes that the “always visible” description should tell Claude:

- what the skill does
- when to use it
- key capabilities
- trigger phrases

So `keywords` alone is not enough. `ms` should support:

- `trigger_phrases: Vec<String>`
- `when_to_use: String`
- compact “what it does” summary

These belong in the route index and candidate projection, not in the always-on prompt.

### 3. Make `ms route` skinny by default

OpenAI’s file search docs explicitly keep raw retrieval results out of the default model response unless the caller opts in. `ms route` should do the same:

- default output: only top candidates + exact next command
- optional `--debug` or `--explain`: raw scores, alternate candidates, retrieval evidence

The default path must optimize for action, not observability.

### 4. Add isolation hints for high-volume skills

Anthropic’s subagent docs recommend isolating verbose operations such as tests, logs, and documentation fetches in separate contexts, returning only summaries. `ms` should carry optional metadata for this:

- `execution_mode: inline | isolated`
- or `subagent_preferred: bool`

This lets compatible agents keep log-heavy or test-heavy skill execution out of the main conversation.

### 5. Optimize for stable prefixes and versioned route schemas

OpenAI’s prompt caching docs say cache hits depend on exact prompt prefixes. Even though `ms` is CLI-first, the same principle improves any API or tool-driven client:

- keep the `AGENTS.md` snippet stable
- keep route JSON field order and top-level schema stable
- add `route_schema_version`
- prefer deterministic `load_command` rendering

This lowers token churn and makes future prompt caching more effective.

### 6. Enforce size discipline on imported skills

Anthropic’s current skill guide recommends keeping `SKILL.md` focused, moving detailed docs to `references/`, and keeping the main skill file under roughly 5,000 words. `ms` should lint for this on import:

- warn if imported `SKILL.md` is too large
- suggest moving long material to `references/`
- preserve references, but do not eagerly surface them

This is one of the cleanest ways to keep lazy loading honest.

---

## User Workflow

### Required Human Steps

1. Run `ms init`.
2. Paste the `ms` blurb into `AGENTS.md`.

Nothing else should be required for the standard case.

### Allowed Optional Human Steps

- Delete provider skill folders after successful import.
- Re-run `ms init --force` or future `ms providers sync` if new provider skills are installed later.

---

## Agent Workflow

For any non-trivial task:

1. Read `AGENTS.md`.
2. Run `ms route "<current task>" -O json`.
3. If a candidate returns `load_command`, run that exact command.
4. If the first load is insufficient, use the returned rehydrate hints or next candidate.
5. Only fall back to `ms search`, `ms list`, or `ms show` if `ms route` has no strong result.

This is the key behavior change: **the agent does not browse the whole skill universe at startup; it asks `ms` to narrow the universe first.**

---

## AGENTS.md Contract

The final `AGENTS.md` snippet must be short enough that users are willing to keep it, and agents can cheaply reread it every session.

### Target snippet

```md
Use `ms` before non-trivial work.

1. Run `ms route "<current task>" -O json`.
2. If a candidate returns a `load_command`, run it exactly.
3. Only use `ms search`, `ms list`, or `ms show` if `ms route` returns no strong match.
```

### Rules

- No large feature overview.
- No long command catalog.
- No startup manifest pasted into `AGENTS.md`.
- No dependency on generated repo-root `SKILL.md`.

`SKILL.md` generation can still exist as an optional integration aid, but it is not the primary workflow anymore.

---

## P0: Critical Gaps

### GAP 1: `ms init` Snapshot Import Is Shipped But Needs Cleanup Verification

**Current state:** `ms init` already initializes local infrastructure and ingests discovered provider skills into the archive/runtime registry.

**Why this still matters:** This is the foundation of the route-first workflow, so the remaining work is to verify the behavior stays correct under collisions, source-folder deletion, and provider relabeling cleanup.

**Current implementation:** `init_human()` and `init_robot()` already follow local initialization with provider discovery, import, archive write, DB upsert, and search indexing.

### Required behavior

During `ms init`, `ms` already:

1. Discover known provider roots, including local-project and home-directory variants.
2. Find all `SKILL.md` files under those roots.
3. Parse each skill into `SkillSpec`.
4. Copy or normalize associated assets (`scripts/`, `references/`, tests if relevant).
5. Persist the imported skill bundle into `.ms/archive`.
6. Upsert the runtime DB/search index from the archived snapshot.
7. Record provenance showing the original provider path and import timestamp.

### Conflict resolution for duplicate skill IDs

Provider collisions must be deterministic from day one.

- Internal canonical ID format: `<provider>/<skill-id>`
- Examples:
  - `claude/rust-error-handling`
  - `codex/rust-error-handling`
  - `agents/rust-error-handling`

Rules:

1. The archive, DB, and route index store canonical provider-qualified IDs.
2. `ms route` may return a short display ID only when it is unambiguous.
3. If multiple providers export the same `skill-id`, route output must use canonical IDs and canonical `load_command` values.
4. Provenance must always include the provider and original source path.

### Suggested provider roots

- `./.claude/skills`
- `~/.claude/skills`
- `./.agents/skills`
- `~/.agents/skills`
- `./.codex/skills`
- `~/.codex/skills`
- `./.gemini/skills`
- `~/.gemini/skills`

This list can be configurable, but it must have strong defaults.

### Remaining implementation direction

1. Keep [`src/cli/commands/init.rs`](/data/projects/meta_skill/src/cli/commands/init.rs:1) as the single bootstrap entry point for provider discovery/import.
2. Verify provider labels remain canonical and unambiguous across `.agents/skills` and `.codex/skills`.
3. Preserve scripts and references so archive-backed behavior survives source-folder deletion.
4. Keep the runtime DB and search index archive-first rather than source-path dependent.
5. Persist `archive_format_version` and `provider` provenance on every imported record.

### Verification

```bash
ms init
find .ms/archive/skills/by-id -maxdepth 2 -type f | head
ms list -O json | jq '.count'
```

Then delete a provider folder and verify:

```bash
mv ~/.claude/skills ~/.claude/skills.bak
ms show some-imported-skill -O json
ms load some-imported-skill --level overview -O json
```

Expected: both commands still work.

---

### GAP 2: `ms route` Must Become The Agent Entry Point

**Problem:** Today the agent has to choose among `search`, `list`, `show`, `suggest`, and `load`. That decision itself burns tokens and causes inconsistent behavior.

**Solution:** Add `ms route "<task>"` as the single compact decision API for agents.

### Required output shape

`ms route` should return compact JSON like:

```json
{
  "route_schema_version": 1,
  "task": "fix rust build error in tokio cli",
  "threshold": 0.65,
  "decision": "match",
  "candidates": [
    {
      "skill_id": "claude/rust-error-handling",
      "display_id": "rust-error-handling",
      "score": 0.93,
      "why": ["project_type:rust", "keyword:error", "signal:thiserror"],
      "when_to_use": "Compiler errors, runtime failures, and error-type design",
      "default_load": "section:checklist",
      "entry_sections": ["checklist", "pitfalls", "examples"],
      "load_command": "ms load claude/rust-error-handling --section checklist -O json"
    }
  ],
  "fallback": {
    "search_command": "ms search \"fix rust build error in tokio cli\" -O json"
  }
}
```

`no_match` case:

```json
{
  "route_schema_version": 1,
  "task": "invent brand new wasm benchmarking rubric",
  "threshold": 0.65,
  "decision": "no_match",
  "candidates": [],
  "fallback": {
    "search_command": "ms search \"invent brand new wasm benchmarking rubric\" -O json",
    "suggest_command": "ms suggest --cwd . -O json"
  }
}
```

`suggest_command` is optional fallback enrichment. `search_command` is the required fallback; `suggest_command` should only be emitted if the command is available in the current build.

### Rules

- Return only top `N` candidates, default `N=3`.
- Return exact `load_command` strings so the agent does not improvise.
- Keep `why` short and enumerable.
- Do not dump the full skill registry.
- Do not return raw retrieval internals by default.
- Add `route_schema_version` so clients can depend on a stable contract.
- Include a route threshold and explicit `decision = match | no_match`.
- Use canonical provider-qualified IDs whenever the short ID is ambiguous.

### Implementation direction

1. Add `route` CLI command under `src/cli/commands/`.
2. Add matching MCP tool in [`src/cli/commands/mcp.rs`](/data/projects/meta_skill/src/cli/commands/mcp.rs:391).
3. Reuse project detection, tool detection, file-pattern matching, and future keyword metadata.
4. Produce deterministic compact output for robot mode.
5. Add `--debug` or `--explain` mode for retrieval evidence rather than bloating the default path.
6. Add negative-result caching with a short TTL for repeated `no_match` tasks.

### MCP schema skeleton

Input:

```json
{
  "type": "object",
  "properties": {
    "task": { "type": "string" },
    "cwd": { "type": "string" },
    "limit": { "type": "integer", "default": 3 },
    "threshold": { "type": "number", "default": 0.65 },
    "debug": { "type": "boolean", "default": false }
  },
  "required": ["task"]
}
```

Output:

```json
{
  "route_schema_version": 1,
  "task": "string",
  "threshold": 0.65,
  "decision": "match|no_match",
  "candidates": [],
  "fallback": {}
}
```

### Verification

```bash
ms route "fix rust build error in tokio cli" -O json | jq '.candidates[0]'
```

Expected: candidate list plus exact minimal `load_command`.

---

### GAP 3: Section-Level Loading Must Exist So Routing Can Stay Cheap

**Problem:** Routing only saves tokens if the follow-up load is also minimal.

**Solution:** Add `--section` loading and ensure the router can recommend section-level entry points.

### Required behavior

- `ms load <skill> --section <slug>`
- `ms route` may return `default_load = "section:<slug>"`
- `ms show -O json` should expose available section slugs

### Slug convention

Section slugs are **kebab-case**.

- `"Getting Started"` -> `getting-started`
- `"Common Pitfalls"` -> `common-pitfalls`
- `"Rust/Async Notes"` -> `rust-async-notes`

Rules:

1. Lowercase ASCII only.
2. Separator is `-`, never `_`.
3. Adjacent separators collapse to one `-`.
4. Leading/trailing separators are trimmed.
5. Empty result is invalid.

### Implementation direction

1. Add `DisclosureLevel::Section`.
2. Add stable `SkillSection::slug()`.
3. Add `sanitize_slug()`.
4. Render only the requested section into `DisclosedContent`.
5. Expose `section_slugs` from `show` and/or route metadata.

### Verification

```bash
ms show rust-error-handling -O json | jq '.section_slugs'
ms load rust-error-handling --section checklist -O json
```

---

## P1: Important Gaps

### GAP 4: Routing Metadata Must Be Explicit

**Problem:** Description text is overloaded. It is too weak and too expensive to be the only routing signal.

**Solution:** Add explicit routing metadata to skill metadata and surface it only in the runtime registry, not in the startup prompt.

### Minimum metadata

Add to `ContextTags` or equivalent routing layer:

- `keywords: Vec<String>`
- `trigger_phrases: Vec<String>`
- `when_to_use: Option<String>`
- `entry_sections: Vec<String>` or derive them deterministically
- `always_on: bool` only if still useful for some thin global skills
- `execution_mode: inline | isolated` or `subagent_preferred: bool`

### Important design note

The agent does **not** need every skill’s routing metadata in prompt context.
`ms` needs it internally so that `ms route` can compute the answer.

### `always_on` behavior

`always_on` does **not** mean “inject into prompt at startup”.

It means:

1. the skill gets a routing prior boost inside `ms route`
2. the skill may appear in candidate sets even under weaker task signals
3. the skill is still delivered only through normal route/load responses

If this behavior proves unnecessary during implementation, remove the field rather than reintroducing prompt injection.

### Verification

```bash
ms route "debug rust panic in cli" -O json
```

Expected: routing decision improves without the agent first calling `search`.

---

### GAP 5: Runtime Registry Must Be Built From Snapshot, Not Live Provider Paths

**Problem:** If the DB/index continues to depend on source directories, deleting provider folders will silently break the promised UX.

**Solution:** After import, the runtime registry should be derived from `.ms/archive` and persisted DB rows, not from the original provider directories.

### Required behavior

- `list`, `show`, `load`, and `route` work after provider directory deletion.
- Re-indexing normal runtime state should use the archive snapshot by default.
- Provider rescans become an explicit sync operation, not an implicit runtime dependency.
- Archive records carry `archive_format_version` for future migrations.

### Migration rule

Archive format changes must be forward-planned:

1. every archived skill record stores `archive_format_version`
2. new readers support the current version and at least one previous version when feasible
3. if migration is required, `ms providers sync` or a dedicated migration command upgrades the snapshot explicitly

### Verification

Remove or rename a provider source directory after init and ensure normal AI workflows still succeed.

---

### GAP 6: Loaded Content Cache Should Cache Archived Skill Content

**Problem:** Even with snapshotting, repeated `load` calls still re-read the same archived content.

**Solution:** Keep the content cache idea, but bind it to archived skill content and archive mtimes/hashes rather than live provider files.

### Verification

```bash
time ms load rust-error-handling --level overview
time ms load rust-error-handling --level overview
```

Expected: second load is materially faster.

---

## P1: Additional Operational Gaps

### GAP 9: Incremental Provider Resync

**Problem:** `ms init --force` is too heavy as the normal path for newly installed provider skills.

**Solution:** Promote provider resync to a normal operational workflow, not a distant follow-up.

Minimum commands:

```bash
ms providers sync
ms providers list
ms providers doctor
```

Required behavior:

- sync only changed/new provider skills when possible
- sync uses per-skill content hash (`Blake3`, full folder walk, path-sorted) stored at import time
- re-import a provider skill only when its stored hash differs from the current provider snapshot hash
- preserve canonical IDs and provenance
- report collisions and archive migrations explicitly

Minimum `ms providers doctor` output should include:

- discovered provider roots
- missing/unreadable provider roots
- archive checksum/integrity status
- DB/archive consistency status
- last sync timestamp

### GAP 10: Minimal Test Strategy

Add explicit tests during implementation:

1. unit tests for canonical ID generation and collision handling
2. unit tests for kebab-case slug generation
3. unit tests for `route` threshold / `no_match` behavior
4. integration test: `ms init` imports provider skills into archive
5. integration test: delete provider folder, then `show/load/route` still work
6. integration test: route returns canonical IDs when collisions exist
7. integration test: add a new skill to a provider folder, run `ms providers sync`, and verify it appears in route results without full re-init

This project is Rust-first; these should live in normal `#[cfg(test)]` modules and `tests/` integration coverage.

### GAP 11: Negative Result Caching

Cache `no_match` route results with a short TTL to avoid repeated full matching on the same low-signal task.

Rules:

- cache key includes normalized task text + cwd fingerprint
- TTL is configurable, default `300s`
- any provider sync or archive mutation invalidates the cache

---

## P2: Nice-to-Have

### GAP 7: Post-Use Compression

Keep `ms compress`, but position it as an optional agent optimization after the core route/load workflow works.

### GAP 8: Import Lint And Size Budget

On import or sync:

- warn when `SKILL.md` exceeds the recommended main-body budget
- suggest moving long docs into `references/`
- derive compact route metadata from descriptions and trigger phrases

---

## Future Follow-up

Keep future provider-management UX improvements here after the core route/archive workflow is stable.

---

## Implementation Order

```
Step 1: Harden shipped init bootstrap
├── Keep ms init provider discovery/import path stable
├── Preserve scripts/references in snapshot form
├── Fix provider-label edge cases and provenance clarity
└── Verify: imported skills survive source-folder deletion

Step 2: Harden registry independence early
├── Make runtime registry archive-first
├── Add archive_format_version
├── Remove hidden dependence on provider source paths
└── Verify: provider folders can be deleted safely before building higher layers

Step 3: Refine shipped route-first agent workflow
├── Keep ms route CLI command stable
├── Keep MCP route tool stable
├── Preserve compact robot output schema
├── Preserve route_schema_version
├── Preserve threshold + no_match path
├── Fix canonical ID/load_command handling under collisions
└── Verify: route returns exact load_command values

Step 4: Keep minimal follow-up loading intact
├── Preserve section slugs
├── Preserve --section loading
├── Expose section slugs via show/route
└── Verify: route -> load(section) works end-to-end

Step 5: Add operational durability
├── Add providers sync/list/doctor
├── Add negative-result cache
├── Add explicit migration handling
└── Verify: incremental updates work without full re-init

Step 6: Optimize hot path
├── Add archive-backed content cache
├── Add import lint for oversized skills
├── Add optional compression
└── Verify: repeated route/load cycles are cheap
```

---

## Success Metrics

| Metric | Target |
|--------|--------|
| Human setup steps | `ms init` + paste short `AGENTS.md` snippet |
| Dependency on provider folders after init | None for normal route/show/load/list flows |
| `AGENTS.md` snippet size | ≤ 8 lines, ≤ 500 characters preferred |
| Agent startup skill catalog in prompt | Zero full catalog injection |
| Route output size | ≤ 3 candidates, ≤ 4.8 KB UTF-8 response body by default (~1200 tokens at `cl100k`) |
| Route default verbosity | No raw scores / retrieval dump unless `--debug` |
| Follow-up load size | section/overview by default, not full |
| Ability to delete provider folders | Preserved |
| Archive completeness | skill body + scripts + references preserved with checksum/integrity verification at import time |
| Imported skill main-body size | Lint warning above recommended budget |

---

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Imported snapshot loses scripts/references | High | Extend archive persistence format before claiming source-folder independence |
| Provider skill ID collisions | Medium | Use canonical provider-qualified IDs internally and short display IDs only when unambiguous |
| `ms route` becomes verbose and defeats token goal | Medium | Strict compact JSON schema and capped candidate count |
| Agent bypasses route and uses search directly | Medium | Keep AGENTS.md snippet route-first and return exact load commands |
| Runtime still secretly depends on provider source paths | High | Add deletion-based integration tests as a release gate |
| AGENTS.md snippet drifts longer over time | Medium | Treat snippet size as a product metric, not just docs text |
| Imported skills are too large to lazy-load well | High | Lint oversized `SKILL.md`, prefer `references/`, and keep route projection thin |
| High-volume skills still flood main context | Medium | Add `execution_mode` / `subagent_preferred` metadata for compatible agents |

---

## What This Plan Explicitly Rejects

- Requiring users to keep provider skill folders forever
- Requiring users to run `ms config skill_paths...` for the normal case
- Requiring agents to read a large generated `SKILL.md` at startup
- Injecting a full all-skills manifest into every session
- Making `search` the primary AI entry point

---

## External References

These sources informed the lazy-loading and route-first decisions in this plan:

- Anthropic, *The Complete Guide to Building Skills for Claude*
  https://resources.anthropic.com/hubfs/The-Complete-Guide-to-Building-Skill-for-Claude.pdf
- Anthropic, *Claude Code Subagents*
  https://docs.anthropic.com/en/docs/claude-code/sub-agents
- Anthropic, *Claude Code MCP*
  https://docs.anthropic.com/en/docs/claude-code/mcp
- OpenAI, *Prompt Caching*
  https://platform.openai.com/docs/guides/prompt-caching
- OpenAI, *File Search Tool*
  https://platform.openai.com/docs/guides/tools-file-search/

### Key takeaways to preserve during implementation

1. Keep always-on prompt surfaces small and stable.
2. Separate routing metadata from fully loaded skill content.
3. Defer verbose payloads until the caller explicitly asks.
4. Prefer deterministic tool outputs and exact next-step commands.
5. Move bulky guidance into lazily loaded references instead of core skill bodies.

---

## Changelog

### Round 3 → Round 4

| Issue | Change |
|-------|--------|
| Plan assumed `SKILL.md` bootstrap was the main surface | Replaced with short `AGENTS.md` contract |
| Plan centered on manifest-first discovery | Replaced with `route`-first AI workflow |
| User still had to configure/index manually | Reframed around self-sufficient `ms init` |
| Provider folders could still be runtime dependencies | Made archive snapshot the required source of truth |
| “Agent knows all skills” implied large prompt payload | Clarified that `ms` holds the full registry and returns only task-relevant slices |
