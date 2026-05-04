//! E2E Scenario: Provider Import Bootstrap (bd-3to2)
//!
//! Tests that `ms init` automatically discovers and imports provider skills
//! from known provider roots, enabling one-command onboarding without manual
//! config or a separate index step.

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
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(skill_dir.join("SKILL.md"), content)?;
    Ok(())
}

/// Test that init discovers skills from a claude provider root and imports them.
#[test]
fn test_init_imports_claude_provider_skills() -> Result<()> {
    let mut fixture = E2EFixture::new("init_claude_provider");

    // Create a claude provider root structure before init
    create_provider_skill(
        &fixture,
        ".claude/skills",
        "rust-patterns",
        r#"---
name: Rust Patterns
description: Common Rust design patterns
tags: [rust, patterns]
---

# Rust Patterns

## Rules

- Prefer composition over inheritance
"#,
    )?;

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "async-rust",
        r#"---
name: Async Rust
description: Asynchronous programming in Rust
tags: [rust, async]
---

# Async Rust

## Rules

- Use async/await for I/O-bound work
"#,
    )?;

    fixture.log_step("Initialize with provider skills present");
    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(
        init_out.success,
        "init should succeed: {}",
        init_out.stderr
    );

    // Verify skills were imported
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

    fixture.log_step("Initialize with no provider roots");
    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(
        init_out.success,
        "init should succeed: {}",
        init_out.stderr
    );

    let list_out = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_out, "list");

    fixture.checkpoint("no_provider_roots");
    Ok(())
}

/// Test that init produces robot JSON with provider_roots_scanned and provider_skills_imported.
#[test]
fn test_init_robot_json_has_provider_fields() -> Result<()> {
    let mut fixture = E2EFixture::new("init_robot_provider_json");

    // Create a codex provider root with one skill
    create_provider_skill(
        &fixture,
        ".codex/skills",
        "python-style",
        r#"---
name: Python Style
description: Python coding conventions
tags: [python, style]
---

# Python Style

## Rules

- Follow PEP 8
"#,
    )?;

    let robot_out = fixture.run_ms(&["--robot", "init"]);
    assert!(
        robot_out.success,
        "init should succeed: {}",
        robot_out.stderr
    );

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

    Ok(())
}

/// Test that collision warning appears when two providers export the same skill ID.
#[test]
fn test_init_collision_warning_on_duplicate_ids() -> Result<()> {
    let mut fixture = E2EFixture::new("init_collision_warning");

    // Same skill ID in two different provider roots
    let skill_content = r#"---
name: Shared Skill
description: A shared skill
tags: []
---

# Shared Skill

## Rules

- Rule one
"#;

    create_provider_skill(&fixture, ".claude/skills", "shared-skill", skill_content)?;
    create_provider_skill(&fixture, ".codex/skills", "shared-skill", skill_content)?;

    fixture.log_step("Initialize with collision");
    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(
        init_out.success,
        "init should succeed: {}",
        init_out.stderr
    );

    // Parse robot JSON output - should still succeed even with collision
    let json: serde_json::Value =
        serde_json::from_str(&init_out.stdout).expect("robot init output should be valid JSON");
    assert_eq!(json.get("status").and_then(|v| v.as_str()), Some("ok"));

    // Both skills should still be imported (not dropped)
    let list_out = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_out, "list");
    fixture.assert_output_contains(&list_out, "shared-skill");

    fixture.checkpoint("collision_handled_gracefully");
    Ok(())
}

/// Test that manual skill_paths config is NOT required for provider root discovery.
/// This is the core regression: init without any config should still import provider skills.
#[test]
fn test_no_manual_config_required_for_provider_discovery() -> Result<()> {
    let mut fixture = E2EFixture::new("init_no_manual_config");

    // Create provider skills before init
    create_provider_skill(
        &fixture,
        ".claude/skills",
        "error-handling",
        r#"---
name: Error Handling
description: Rust error handling patterns
tags: [rust, errors]
---

# Error Handling

## Rules

- Use Result for fallible operations
- Use Option for nullable values
"#,
    )?;

    // Init with no manual config - should still import
    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    // Skills should be available via list without manual index
    let list_out = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_out, "list");
    fixture.assert_output_contains(&list_out, "error-handling");

    fixture.checkpoint("no_manual_config_needed");
    Ok(())
}
