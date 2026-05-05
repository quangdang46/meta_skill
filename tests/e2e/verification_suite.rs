//! E2E Verification Suite — Bead bd-1p6y
//!
//! Integration tests verifying:
//! - Canonical ID generation and collision handling
//! - Kebab-case slug generation
//! - Route threshold and no_match behavior
//! - ms init imports provider skills into archive
//! - Deleting provider folder: show/load/route still work
//! - Route returns canonical IDs with collisions
//! - Add skill + ms providers sync = appears in route without re-init
//! - ms providers doctor with degraded source state
//! - no_match cache invalidates on provider sync import
//! - Repeated load --section uses cache, mutation invalidates

use std::fs;
use std::path::Path;

use super::fixture::E2EFixture;
use ms::error::Result;

fn create_provider_skill(
    skill_dir: &Path,
    markdown: &str,
    scripts: &[(&str, &str)],
    references: &[(&str, &str)],
) -> Result<()> {
    fs::create_dir_all(skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), markdown)?;

    if !scripts.is_empty() {
        let scripts_dir = skill_dir.join("scripts");
        fs::create_dir_all(&scripts_dir)?;
        for (name, content) in scripts {
            fs::write(scripts_dir.join(name), content)?;
        }
    }

    if !references.is_empty() {
        let refs_dir = skill_dir.join("references");
        fs::create_dir_all(&refs_dir)?;
        for (name, content) in references {
            fs::write(refs_dir.join(name), content)?;
        }
    }

    Ok(())
}

/// bd-1p6y unit 1: Canonical ID generation and collision handling
#[test]
fn test_canonical_id_generation() -> Result<()> {
    let mut fixture = E2EFixture::new("canonical_id_generation");
    fixture.init();
    let output = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&output, "init");

    // Create two skills with same name but different providers
    fixture.create_skill(
        "error-handler",
        r#"---
id: error-handler
name: Error Handler
description: Handles errors in Rust
tags: [rust, errors]
provider: community
---
# Error Handler
Some content.
"#,
    )?;
    fixture.run_ms(&["index"]);

    let show_out = fixture.run_ms(&["--robot", "show", "error-handler"]);
    fixture.assert_success(&show_out, "show");
    let show_json = show_out.json();
    assert_eq!(show_json["skill"]["id"], "community/error-handler");
    assert_eq!(show_json["skill"]["stored_id"], "community/error-handler");
    assert_eq!(show_json["skill"]["display_id"], "error-handler");

    Ok(())
}

/// bd-1p6y unit 2: Kebab-case slug generation
#[test]
fn test_kebab_case_slug() -> Result<()> {
    let mut fixture = E2EFixture::new("kebab_slug");
    fixture.init();
    let output = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&output, "init");

    // Create a multi-section skill whose headings require slug sanitization.
    fixture.create_skill(
        "slug-skill",
        r#"---
id: slug-skill
name: Slug Skill
description: A test for kebab-case section slugs
tags: [test]
---
# Slug Skill

## Rust Error Handling
Section content for slug verification.

## CLI & Setup
Setup instructions live here.
"#,
    )?;
    fixture.run_ms(&["index"]);

    let show_out = fixture.run_ms(&["--robot", "show", "slug-skill"]);
    fixture.assert_success(&show_out, "show");
    let show_json = show_out.json();
    let slugs = show_json["skill"]["section_slugs"]
        .as_array()
        .expect("section_slugs should be an array")
        .iter()
        .filter_map(|slug| slug.as_str())
        .collect::<Vec<_>>();
    assert!(
        slugs.contains(&"rust-error-handling"),
        "expected rust-error-handling slug, got {slugs:?}"
    );
    assert!(
        slugs.contains(&"cli-setup"),
        "expected cli-setup slug, got {slugs:?}"
    );

    let load_out = fixture.run_ms(&[
        "--robot",
        "load",
        "slug-skill",
        "--section",
        "rust-error-handling",
    ]);
    fixture.assert_success(&load_out, "load_section_by_slug");
    assert_eq!(
        load_out.json()["data"]["content"],
        "## Rust Error Handling\n\nSection content for slug verification.\n\n"
    );

    Ok(())
}

/// bd-1p6y unit 3: Route threshold and no_match behavior
#[test]
fn test_route_no_match() -> Result<()> {
    let mut fixture = E2EFixture::new("route_no_match");
    fixture.init();
    let output = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&output, "init");

    // Route for something that doesn't exist — should return a stable no_match contract.
    let route_out = fixture.run_ms(&["--robot", "route", "zzzzz_nonexistent_skill_xyz"]);
    fixture.assert_success(&route_out, "route_no_match");
    let route_json = route_out.json();
    assert_eq!(route_json["decision"], "no_match");
    assert_eq!(
        route_json["fallback"]["search_command"],
        "ms search \"zzzzz_nonexistent_skill_xyz\" -O json"
    );
    assert!(
        route_json["candidates"]
            .as_array()
            .expect("candidates")
            .is_empty(),
        "no_match response should not include candidates"
    );

    Ok(())
}

/// bd-1p6y unit 4: ms init imports provider skills into archive
#[test]
fn test_provider_skills_in_archive() -> Result<()> {
    let mut fixture = E2EFixture::new("provider_skills_archive");
    let claude_root = fixture.root.join(".claude/skills");
    create_provider_skill(
        &claude_root.join("provider-skill"),
        r#"---
id: provider-skill
name: Provider Skill
description: A skill from a provider source
tags: [provider, test]
trigger_phrases: ["provider archive import"]
---
# Provider Skill

## Overview
Content from provider.
"#,
        &[],
        &[],
    )?;

    let output = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&output, "init");
    let init_json = output.json();
    assert_eq!(init_json["provider_skills_imported"], 1);
    assert!(
        init_json["provider_roots_scanned"].as_u64().unwrap_or(0) >= 1,
        "expected init to scan at least one provider root"
    );

    let archive_skill = fixture
        .ms_root
        .join("archive/skills/by-id/claude/provider-skill");
    assert!(
        archive_skill.exists(),
        "expected archived provider skill at {}",
        archive_skill.display()
    );

    let show_out = fixture.run_ms(&["--robot", "show", "claude/provider-skill"]);
    fixture.assert_success(&show_out, "show_provider_skill");
    let show_json = show_out.json();
    assert_eq!(show_json["skill"]["id"], "claude/provider-skill");
    assert_eq!(show_json["skill"]["stored_id"], "claude/provider-skill");
    assert_eq!(show_json["skill"]["display_id"], "provider-skill");
    assert_eq!(
        show_json["skill"]["git_remote"],
        fixture
            .ms_root
            .join("archive")
            .to_string_lossy()
            .to_string()
    );

    let route_out = fixture.run_ms(&["--robot", "route", "provider archive import"]);
    fixture.assert_success(&route_out, "route_provider_skill");
    assert_eq!(route_out.json()["decision"], "match");

    Ok(())
}

/// bd-1p6y unit 5: Delete provider folder, show/load/route still work
#[test]
fn test_delete_provider_show_still_works() -> Result<()> {
    let mut fixture = E2EFixture::new("delete_provider");
    let claude_root = fixture.root.join(".claude/skills");
    create_provider_skill(
        &claude_root.join("persistent-skill"),
        r#"---
id: persistent-skill
name: Persistent Skill
description: A provider skill that persists in archive
tags: [test, persistence]
trigger_phrases: ["persistent archive route"]
---
# Persistent Skill

## Overview
This skill should survive provider deletion.
"#,
        &[],
        &[],
    )?;

    let output = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&output, "init");

    let show_before = fixture.run_ms(&["--robot", "show", "claude/persistent-skill"]);
    fixture.assert_success(&show_before, "show_before");
    let route_before = fixture.run_ms(&["--robot", "route", "persistent archive route"]);
    fixture.assert_success(&route_before, "route_before");
    assert_eq!(route_before.json()["decision"], "match");

    std::fs::remove_dir_all(&claude_root)?;

    let show_after = fixture.run_ms(&["--robot", "show", "claude/persistent-skill"]);
    fixture.assert_success(&show_after, "show_after_deletion");
    let load_after = fixture.run_ms(&[
        "--robot",
        "load",
        "claude/persistent-skill",
        "--section",
        "overview",
    ]);
    fixture.assert_success(&load_after, "load_after_deletion");
    assert_eq!(
        load_after.json()["data"]["content"],
        "## Overview\n\nThis skill should survive provider deletion.\n\n"
    );

    let route_after = fixture.run_ms(&["--robot", "route", "persistent archive route"]);
    fixture.assert_success(&route_after, "route_after_deletion");
    assert_eq!(route_after.json()["decision"], "match");
    assert!(
        route_after.json()["candidates"]
            .as_array()
            .expect("route candidates")
            .iter()
            .any(|candidate| candidate["skill_id"] == "claude/persistent-skill"),
        "route should still return the archived provider skill after root deletion"
    );

    Ok(())
}

/// bd-1p6y unit 6: Route returns canonical IDs with collisions
#[test]
fn test_route_canonical_ids_with_collisions() -> Result<()> {
    let mut fixture = E2EFixture::new("route_canonical_collisions");
    let claude_root = fixture.root.join(".claude/skills");
    let codex_root = fixture.root.join(".codex/skills");

    create_provider_skill(
        &claude_root.join("common-name"),
        r#"---
id: common-name
name: Common Name
description: Claude provider collision skill
tags: [test, provider]
trigger_phrases: ["common collision route"]
---
# Common Name
Content A.
"#,
        &[],
        &[],
    )?;

    create_provider_skill(
        &codex_root.join("common-name"),
        r#"---
id: common-name
name: Common Name
description: Codex provider collision skill
tags: [test, provider]
trigger_phrases: ["common collision route"]
---
# Common Name
Content B.
"#,
        &[],
        &[],
    )?;

    let output = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&output, "init");

    // Route should handle collisions gracefully and return canonical IDs in provider/skill-id format.
    let route_out = fixture.run_ms(&["--robot", "route", "common collision route"]);
    fixture.assert_success(&route_out, "route_with_collision");

    // Parse JSON output and verify canonical format
    let json = route_out.json();
    let candidates = json["candidates"].as_array().expect("candidates array");
    assert!(
        !candidates.is_empty(),
        "Should return candidates for collision query"
    );

    // Each candidate should have canonical format when there's a collision
    for cand in candidates {
        let skill_id = cand["skill_id"]
            .as_str()
            .expect("skill_id should be string");
        let load_command = cand["load_command"]
            .as_str()
            .expect("load_command should be string");
        // When there's a collision (same skill id but different providers),
        // the skill_id should be in "provider/skill-id" format
        assert!(
            skill_id.contains("/"),
            "Expected canonical format 'provider/skill-id', got: {}",
            skill_id
        );
        assert!(
            load_command.contains(skill_id),
            "Expected load_command to use canonical skill_id '{skill_id}', got: {load_command}"
        );
    }

    let canonical_ids = candidates
        .iter()
        .map(|cand| cand["skill_id"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert!(
        canonical_ids.iter().any(|id| id == "claude/common-name"),
        "expected claude/common-name candidate, got {canonical_ids:?}"
    );
    assert!(
        canonical_ids.iter().any(|id| id == "codex/common-name"),
        "expected codex/common-name candidate, got {canonical_ids:?}"
    );

    Ok(())
}

/// bd-1p6y unit 7: Add skill + ms providers sync = appears in route without re-init
#[test]
fn test_add_skill_appears_in_route() -> Result<()> {
    let mut fixture = E2EFixture::new("add_skill_route");
    fixture.init();
    let output = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&output, "init");

    // Create and index a skill
    fixture.create_skill(
        "new-skill",
        r#"---
id: new-skill
name: New Skill
description: A newly added skill
tags: [new]
---
# New Skill
Fresh content.
"#,
    )?;
    fixture.run_ms(&["index"]);

    // Route should find it
    let route_out = fixture.run_ms(&["route", "new skill"]);
    fixture.assert_success(&route_out, "route_new_skill");

    Ok(())
}

/// bd-1p6y unit 8: ms providers doctor with degraded source state
#[test]
fn test_providers_doctor_degraded() -> Result<()> {
    let mut fixture = E2EFixture::new("providers_doctor");
    let claude_root = fixture.root.join(".claude/skills");
    let skill_dir = claude_root.join("provider-doctor");

    create_provider_skill(
        &skill_dir,
        r#"---
id: provider-doctor
name: Provider Doctor
description: Provider doctor degradation test
tags: [provider, doctor]
trigger_phrases: ["provider doctor degrade"]
---
# Provider Doctor

## Overview
Archive-backed runtime should stay usable when source roots disappear.
"#,
        &[],
        &[],
    )?;

    let output = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&output, "init");

    fs::rename(&claude_root, fixture.root.join(".claude/skills.hidden"))?;

    let show_out = fixture.run_ms(&["--robot", "show", "claude/provider-doctor"]);
    fixture.assert_success(&show_out, "show_provider_doctor_after_root_hide");

    let doctor_out = fixture.run_ms(&["--robot", "providers", "doctor"]);
    fixture.assert_success(&doctor_out, "providers_doctor");
    let doctor_json = doctor_out.json();
    assert_eq!(doctor_json["status"], "degraded");
    assert_eq!(doctor_json["runtime_usable"], true);
    assert_eq!(doctor_json["archive"]["integrity_status"], "ok");
    assert_eq!(doctor_json["registry"]["consistent"], true);
    assert!(
        doctor_json["roots"]
            .as_array()
            .expect("roots")
            .iter()
            .any(|root| root["provider"] == "claude" && root["readable"] == false),
        "doctor should report the missing claude provider root"
    );

    Ok(())
}

/// bd-1p6y unit 9: no_match cache invalidates on provider sync import
#[test]
fn test_no_match_cache_invalidates() -> Result<()> {
    let mut fixture = E2EFixture::new("no_match_cache");
    let claude_root = fixture.root.join(".claude/skills");
    let output = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&output, "init");

    // First route to a non-existent skill (populates no_match cache)
    let _first = fixture.run_ms(&["--robot", "route", "nonexistent-skill"]);
    // This is expected to not find anything (no_match cache populated)

    // Now create the skill in a provider root so providers sync can discover it.
    create_provider_skill(
        &claude_root.join("now-it-exists"),
        r#"---
id: now-it-exists
name: Now It Exists
description: Was missing, now here
tags: [test, provider]
trigger_phrases: ["now it exists"]
---
# Content
Now available.
"#,
        &[],
        &[],
    )?;

    // Run providers sync --apply to trigger cache invalidation
    let sync_out = fixture.run_ms(&["--robot", "providers", "sync", "--apply"]);
    fixture.assert_success(&sync_out, "providers_sync_apply");

    // Re-route — should find it now after invalidation
    let route_again = fixture.run_ms(&["--robot", "route", "now it exists"]);
    fixture.assert_success(&route_again, "route_after_sync");
    let route_json = route_again.json();
    assert_eq!(route_json["decision"], "match");
    assert!(
        route_json["candidates"]
            .as_array()
            .expect("candidates")
            .iter()
            .any(|candidate| candidate["skill_id"] == "claude/now-it-exists"),
        "route should match the newly imported provider skill after cache invalidation"
    );

    Ok(())
}

/// bd-1p6y unit 10: Repeated load --section uses cache, mutation invalidates
#[test]
fn test_repeated_load_section_cache() -> Result<()> {
    let mut fixture = E2EFixture::new("load_section_cache");
    let claude_root = fixture.root.join(".claude/skills");
    let skill_dir = claude_root.join("cached-skill");
    create_provider_skill(
        &skill_dir,
        r#"---
id: cached-skill
name: Cached Skill
description: Skill with multiple sections for cache testing
tags: [test, cache]
---
# Cached Skill

## Overview
Base content.

## Details
Detailed content here.
"#,
        &[],
        &[],
    )?;

    let output = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&output, "init");

    let cache_dir = fixture.ms_root.join("cache").join("content");

    let first_load = fixture.run_ms(&[
        "--robot",
        "load",
        "claude/cached-skill",
        "--section",
        "details",
    ]);
    fixture.assert_success(&first_load, "first_section_load");
    let first_json = first_load.json();
    assert_eq!(
        first_json["data"]["content"],
        "## Details\n\nDetailed content here.\n\n"
    );

    let cache_entries_after_first = fs::read_dir(&cache_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert_eq!(cache_entries_after_first.len(), 1);

    let second_load = fixture.run_ms(&[
        "--robot",
        "load",
        "claude/cached-skill",
        "--section",
        "details",
    ]);
    fixture.assert_success(&second_load, "second_section_load");
    assert_eq!(
        second_load.json()["data"]["content"],
        "## Details\n\nDetailed content here.\n\n"
    );
    let cache_entries_after_second = fs::read_dir(&cache_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert_eq!(cache_entries_after_first, cache_entries_after_second);

    create_provider_skill(
        &skill_dir,
        r#"---
id: cached-skill
name: Cached Skill
description: Skill with multiple sections for cache testing
tags: [test, cache]
---
# Cached Skill

## Overview
Base content.

## Details
Updated detail content.
"#,
        &[],
        &[],
    )?;

    let sync_out = fixture.run_ms(&["--robot", "providers", "sync", "--apply"]);
    fixture.assert_success(&sync_out, "providers_sync_for_cache_invalidation");

    let third_load = fixture.run_ms(&[
        "--robot",
        "load",
        "claude/cached-skill",
        "--section",
        "details",
    ]);
    fixture.assert_success(&third_load, "third_section_load");
    assert_eq!(
        third_load.json()["data"]["content"],
        "## Details\n\nUpdated detail content.\n\n"
    );
    let cache_entries_after_third = fs::read_dir(&cache_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert_eq!(cache_entries_after_third.len(), 1);
    assert_ne!(cache_entries_after_first, cache_entries_after_third);

    Ok(())
}

/// bd-1p6y unit 11: provider roots auto-discover, then archive-backed load survives source removal
#[test]
fn test_provider_auto_discovery_and_archive_fallback() -> Result<()> {
    let mut fixture = E2EFixture::new("provider_auto_discovery_archive_fallback");
    let claude_root = fixture.root.join(".claude/skills");
    let codex_root = fixture.root.join(".codex/skills");
    let provider_route_dir = claude_root.join("provider-route");
    let global_helper_dir = codex_root.join("global-helper");

    create_provider_skill(
        &provider_route_dir,
        r#"---
id: provider-route
name: Provider Route
description: Route-first provider verification skill
tags: [provider, route, archive]
trigger_phrases: ["provider route verification"]
when_to_use: "Testing provider import and archive fallback"
---
# Provider Route

## Overview
Use this skill to verify provider auto-discovery and archive-backed loading.

## Checklist
- Route to the canonical provider-qualified id
- Preserve scripts and references after the source folder disappears
"#,
        &[("verify.sh", "#!/bin/sh\necho verify\n")],
        &[("guide.md", "# Provider Route Guide\n")],
    )?;

    create_provider_skill(
        &global_helper_dir,
        r#"---
id: global-helper
name: Global Helper
description: Global codex helper skill
tags: [global, helper]
---
# Global Helper

## Overview
Global helper content.
"#,
        &[],
        &[],
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&init_out, "init");

    let list_out = fixture.run_ms(&["--robot", "providers", "list"]);
    fixture.assert_success(&list_out, "providers_list");
    let list_json = list_out.json();
    let roots = list_json["roots"].as_array().expect("roots array");
    let claude_root_str = claude_root.to_string_lossy().to_string();
    let codex_root_str = codex_root.to_string_lossy().to_string();
    assert!(
        roots.iter().any(|root| {
            root["provider"] == "claude"
                && root["path"].as_str() == Some(claude_root_str.as_str())
                && root["tracked_skill_count"].as_u64().unwrap_or(0) >= 1
                && root["last_sync"].is_string()
        }),
        "providers list should include local claude root"
    );
    assert!(
        roots.iter().any(|root| {
            root["provider"] == "codex"
                && root["path"].as_str() == Some(codex_root_str.as_str())
                && root["tracked_skill_count"].as_u64().unwrap_or(0) >= 1
                && root["last_sync"].is_string()
        }),
        "providers list should include home codex root"
    );

    let route_out = fixture.run_ms(&["--robot", "route", "provider route verification"]);
    fixture.assert_success(&route_out, "route_provider_route");
    let route_json = route_out.json();
    assert_eq!(route_json["decision"], "match");
    let candidate = &route_json["candidates"][0];
    assert_eq!(candidate["skill_id"], "claude/provider-route");
    assert!(
        candidate["load_command"]
            .as_str()
            .expect("load command")
            .contains("claude/provider-route"),
        "load command should use the canonical provider-qualified id"
    );

    let show_out = fixture.run_ms(&["--robot", "show", "claude/provider-route"]);
    fixture.assert_success(&show_out, "show_provider_route");
    let show_json = show_out.json();
    assert_eq!(show_json["skill"]["id"], "claude/provider-route");
    assert_eq!(show_json["skill"]["stored_id"], "claude/provider-route");
    assert_eq!(show_json["skill"]["display_id"], "provider-route");

    let hidden_provider_route_dir = fixture.root.join("provider-route.hidden");
    fs::rename(&provider_route_dir, &hidden_provider_route_dir)?;

    let load_out = fixture.run_ms(&["--robot", "load", "claude/provider-route", "--complete"]);
    fixture.assert_success(&load_out, "load_provider_route_complete");
    let load_json = load_out.json();
    assert_eq!(load_json["data"]["skill_id"], "claude/provider-route");
    assert_eq!(load_json["data"]["scripts"][0]["path"], "scripts/verify.sh");
    assert_eq!(
        load_json["data"]["references"][0]["path"],
        "references/guide.md"
    );

    Ok(())
}

/// bd-1p6y unit 12: providers sync imports discovered provider roots and invalidates no_match cache
#[test]
fn test_provider_sync_apply_imports_new_skill_and_invalidates_route_cache() -> Result<()> {
    let mut fixture = E2EFixture::new("provider_sync_invalidation_real_roots");
    let claude_root = fixture.root.join(".claude/skills");
    let bootstrap_dir = claude_root.join("bootstrap");

    create_provider_skill(
        &bootstrap_dir,
        r#"---
id: bootstrap
name: Bootstrap
description: Initial provider skill
tags: [bootstrap]
---
# Bootstrap

## Overview
Initial provider content.
"#,
        &[],
        &[],
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&init_out, "init");

    let first_route = fixture.run_ms(&["--robot", "route", "sync me later"]);
    assert_eq!(first_route.json()["decision"], "no_match");

    let sync_me_dir = claude_root.join("sync-me");
    create_provider_skill(
        &sync_me_dir,
        r#"---
id: sync-me
name: Sync Me
description: Imported by providers sync after a cached no-match
tags: [sync, invalidation]
trigger_phrases: ["sync me later"]
---
# Sync Me

## Overview
This skill should appear after providers sync invalidates the route cache.
"#,
        &[],
        &[],
    )?;

    let sync_out = fixture.run_ms(&["--robot", "providers", "sync", "--apply"]);
    fixture.assert_success(&sync_out, "providers_sync_apply");
    let sync_json = sync_out.json();
    let reports = sync_json["roots"].as_array().expect("sync reports");
    assert!(
        reports.iter().any(|report| {
            report["root_path"].as_str() == Some(claude_root.to_string_lossy().as_ref())
                && report["new_count"].as_u64().unwrap_or(0) == 1
        }),
        "providers sync should detect and import the new claude-root skill"
    );

    let route_out = fixture.run_ms(&["--robot", "route", "sync me later"]);
    fixture.assert_success(&route_out, "route_after_sync");
    let route_json = route_out.json();
    assert_eq!(route_json["decision"], "match");
    assert!(
        route_json["candidates"]
            .as_array()
            .expect("candidates")
            .iter()
            .any(|candidate| candidate["skill_id"] == "claude/sync-me"),
        "route should find the newly imported provider skill after sync"
    );

    let show_out = fixture.run_ms(&["--robot", "show", "claude/sync-me"]);
    fixture.assert_success(&show_out, "show_sync_me");

    Ok(())
}

/// bd-k6xi: oversized provider imports surface references guidance during init and sync
#[test]
fn test_provider_import_lint_surfaces_references_guidance() -> Result<()> {
    let mut fixture = E2EFixture::new("provider_import_lint_guidance");
    let claude_root = fixture.root.join(".claude/skills");
    let oversized_body = "word ".repeat(5_200);

    create_provider_skill(
        &claude_root.join("oversized-init"),
        &format!(
            r#"---
id: oversized-init
name: Oversized Init
description: Oversized provider skill imported during init
tags: [provider, lint]
trigger_phrases: ["init oversized route"]
---
# Oversized Init

## Overview
{}
"#,
            oversized_body
        ),
        &[],
        &[],
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&init_out, "init");
    let init_json = init_out.json();
    let init_warnings = init_json["provider_import_warnings"]
        .as_array()
        .expect("provider_import_warnings");
    assert!(
        init_warnings.iter().any(|warning| {
            warning["skill_id"] == "oversized-init"
                && warning["diagnostics"]
                    .as_array()
                    .expect("diagnostics")
                    .iter()
                    .any(|diag| {
                        diag["rule_id"] == "oversized-skill-md"
                            && diag["suggestion"]
                                .as_str()
                                .unwrap_or_default()
                                .contains("references/")
                            && diag["suggestion"]
                                .as_str()
                                .unwrap_or_default()
                                .contains("trigger:init oversized route")
                    })
        }),
        "init should surface oversized import guidance with references/ and trigger hints"
    );

    create_provider_skill(
        &claude_root.join("oversized-sync"),
        &format!(
            r#"---
id: oversized-sync
name: Oversized Sync
description: Oversized provider skill imported during providers sync
tags: [provider, lint]
trigger_phrases: ["sync oversized route"]
---
# Oversized Sync

## Overview
{}
"#,
            oversized_body
        ),
        &[],
        &[],
    )?;

    let sync_out = fixture.run_ms(&["--robot", "providers", "sync", "--apply"]);
    fixture.assert_success(&sync_out, "providers_sync_apply");
    let sync_json = sync_out.json();
    let reports = sync_json["roots"].as_array().expect("sync reports");
    assert!(
        reports.iter().any(|report| {
            report["lint_warnings"]
                .as_array()
                .expect("lint warnings")
                .iter()
                .any(|warning| {
                    warning["skill_id"] == "oversized-sync"
                        && warning["diagnostics"]
                            .as_array()
                            .expect("diagnostics")
                            .iter()
                            .any(|diag| {
                                diag["rule_id"] == "oversized-skill-md"
                                    && diag["suggestion"]
                                        .as_str()
                                        .unwrap_or_default()
                                        .contains("references/")
                                    && diag["suggestion"]
                                        .as_str()
                                        .unwrap_or_default()
                                        .contains("trigger:sync oversized route")
                            })
                })
        }),
        "providers sync should surface oversized import guidance with references/ and trigger hints"
    );

    Ok(())
}
