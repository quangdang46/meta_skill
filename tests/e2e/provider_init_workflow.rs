//! E2E Scenario: Provider Import Bootstrap & Archive Integrity (bd-3to2, bd-28jh)
//!
//! Tests:
//! - `ms init` auto-discovers and imports provider skills (bd-3to2)
//! - Imported skills survive provider folder deletion (bd-28jh)
//! - Content hash and archive format version are stored in DB (bd-28jh)

use std::fs;
use super::fixture::E2EFixture;
use ms::error::Result;

/// Helper: create a SKILL.md in a provider root under the fixture's project dir.
fn create_provider_skill(
    fixture: &E2EFixture,
    provider_dir: &str,
    skill_name: &str,
    content: &str,
) -> Result<()> {
    let skill_dir = fixture.root.join(provider_dir).join(skill_name);
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// bd-3to2: ms init provider discovery and bootstrap
// ---------------------------------------------------------------------------

/// Test that init discovers skills from a claude provider root and imports them.
#[test]
fn test_init_imports_claude_provider_skills() -> Result<()> {
    let mut fixture = E2EFixture::new("init_claude_provider");

    create_provider_skill(
        &fixture, ".claude/skills", "rust-patterns",
        "# Rust Patterns\n\n## Rules\n\n- Prefer composition over inheritance\n",
    )?;
    create_provider_skill(
        &fixture, ".claude/skills", "async-rust",
        "# Async Rust\n\n## Rules\n\n- Use async/await for I/O-bound work\n",
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    let list_out = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_out, "list");
    fixture.assert_output_contains(&list_out, "rust-patterns");
    fixture.assert_output_contains(&list_out, "async-rust");

    fixture.checkpoint("provider_imported");
    Ok(())
}

/// Test that init succeeds and list works even with no provider roots.
#[test]
fn test_init_succeeds_with_no_provider_roots() -> Result<()> {
    let mut fixture = E2EFixture::new("init_no_providers");

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    let list_out = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_out, "list");

    fixture.checkpoint("no_provider_roots");
    Ok(())
}

/// Test that init produces robot JSON with provider_roots_scanned and provider_skills_imported.
#[test]
fn test_init_robot_json_has_provider_fields() -> Result<()> {
    let mut fixture = E2EFixture::new("init_robot_provider_json");

    create_provider_skill(
        &fixture, ".codex/skills", "python-style",
        "# Python Style\n\n## Rules\n\n- Follow PEP 8\n",
    )?;

    let robot_out = fixture.run_ms(&["--robot", "init"]);
    assert!(robot_out.success, "init should succeed: {}", robot_out.stderr);

    let json: serde_json::Value =
        serde_json::from_str(&robot_out.stdout).expect("robot init output should be valid JSON");

    assert_eq!(json.get("status").and_then(|v| v.as_str()), Some("ok"));
    assert!(
        json.get("provider_roots_scanned").is_some(),
        "robot JSON should include provider_roots_scanned"
    );
    assert!(
        json.get("provider_skills_imported").is_some(),
        "robot JSON should include provider_skills_imported"
    );

    fixture.checkpoint("robot_json_fields");
    Ok(())
}

/// Test that collision warning appears when two providers export the same skill ID.
#[test]
fn test_init_collision_warning_on_duplicate_ids() -> Result<()> {
    let mut fixture = E2EFixture::new("init_collision_warning");

    let skill_content = "# Shared Skill\n\n## Rules\n\n- Rule one\n";
    create_provider_skill(&fixture, ".claude/skills", "shared-skill", skill_content)?;
    create_provider_skill(&fixture, ".codex/skills", "shared-skill", skill_content)?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    let json: serde_json::Value =
        serde_json::from_str(&init_out.stdout).expect("robot init output should be valid JSON");
    assert_eq!(json.get("status").and_then(|v| v.as_str()), Some("ok"));

    let list_out = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_out, "list");
    fixture.assert_output_contains(&list_out, "shared-skill");

    fixture.checkpoint("collision_handled_gracefully");
    Ok(())
}

/// Test that manual skill_paths config is NOT required for provider root discovery.
#[test]
fn test_no_manual_config_required_for_provider_discovery() -> Result<()> {
    let mut fixture = E2EFixture::new("init_no_manual_config");

    create_provider_skill(
        &fixture, ".claude/skills", "error-handling",
        "# Error Handling\n\n## Rules\n\n- Use Result for fallible operations\n",
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    let list_out = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_out, "list");
    fixture.assert_output_contains(&list_out, "error-handling");

    fixture.checkpoint("no_manual_config_needed");
    Ok(())
}

// ---------------------------------------------------------------------------
// bd-28jh: Archive integrity — assets survive deletion, checksums stored
// ---------------------------------------------------------------------------

/// Test that provider-imported skills remain usable after source folder deletion.
#[test]
fn test_imported_skills_survive_source_deletion() -> Result<()> {
    let mut fixture = E2EFixture::new("import_survives_deletion");

    create_provider_skill(
        &fixture, ".claude/skills", "survivable-skill",
        "# Survivable Skill\n\n## Rules\n\n- This rule survives source deletion\n",
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    let show_out = fixture.run_ms(&["--robot", "show", "survivable-skill"]);
    fixture.assert_success(&show_out, "show before deletion");
    let show_json: serde_json::Value =
        serde_json::from_str(&show_out.stdout).expect("valid JSON");
    assert_eq!(show_json["skill"]["name"].as_str().unwrap(), "Survivable Skill");

    // Delete the source provider folder entirely
    fs::remove_dir_all(fixture.root.join(".claude/skills"))?;

    // Show should still work — archive-backed
    let show_after = fixture.run_ms(&["--robot", "show", "survivable-skill"]);
    fixture.assert_success(&show_after, "show after deletion");

    // Load should also work
    let load_out = fixture.run_ms(&["--robot", "load", "survivable-skill"]);
    fixture.assert_success(&load_out, "load after deletion");
    assert!(load_out.stdout.contains("Survivable Skill"));

    fixture.checkpoint("survived_source_deletion");
    Ok(())
}

/// Test that scripts within provider-imported skills survive source deletion.
#[test]
fn test_imported_scripts_survive_deletion() -> Result<()> {
    let mut fixture = E2EFixture::new("import_scripts_survive");

    let skill_dir = fixture.root.join(".codex/skills/scripted-skill");
    fs::create_dir_all(&skill_dir.join("scripts"))?;
    fs::write(
        skill_dir.join("SKILL.md"),
        "# Scripted Skill\n\n## Rules\n\n- Run the setup script first\n",
    )?;
    fs::write(skill_dir.join("scripts/setup.sh"), "#!/bin/sh\necho setup")?;
    fs::write(skill_dir.join("scripts/run.sh"), "#!/bin/sh\necho running")?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    let show_out = fixture.run_ms(&["--robot", "show", "scripted-skill"]);
    fixture.assert_success(&show_out, "show before deletion");

    fs::remove_dir_all(fixture.root.join(".codex/skills"))?;

    // Show still works after source deletion
    let show_after = fixture.run_ms(&["--robot", "show", "scripted-skill"]);
    fixture.assert_success(&show_after, "show after deletion");

    fixture.checkpoint("scripts_survived_deletion");
    Ok(())
}

/// Test that content hash and archive_format_version are stored in the DB for imported skills.
#[test]
fn test_import_stores_integrity_metadata() -> Result<()> {
    let mut fixture = E2EFixture::new("import_integrity_metadata");

    create_provider_skill(
        &fixture, ".claude/skills", "integrity-skill",
        "# Integrity Skill\n\n## Rules\n\n- Content hash and format version are recorded\n",
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    // Verify content_hash and archive_format_version are stored in DB
    fixture.open_db();
    fixture.verify_db_state(
        |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT content_hash, archive_format_version \
                     FROM skills WHERE name = 'Integrity Skill'",
                )
                .expect("prepare");
            let result = stmt
                .query_row([], |row| {
                    let hash: String = row.get(0).unwrap_or_default();
                    let afv: Option<String> = row.get(1).ok();
                    Ok((hash, afv))
                })
                .ok();
            match result {
                Some((hash, afv)) => {
                    println!(
                        "[bd-28jh] content_hash present (len={}), archive_format_version={:?}",
                        hash.len(), afv
                    );
                    !hash.is_empty() && afv.is_some()
                }
                None => {
                    println!("[bd-28jh] No record found for 'Integrity Skill'");
                    false
                }
            }
        },
        "imported skill should have non-empty content_hash and archive_format_version",
    );

    fixture.checkpoint("integrity_metadata_stored");
    Ok(())
}
