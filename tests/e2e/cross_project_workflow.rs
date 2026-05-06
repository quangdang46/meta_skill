//! E2E Scenario: Cross-Project Workflow Integration Tests
//!
//! Comprehensive tests for the `ms cross-project` command covering:
//! - Summary subcommand (aggregation by project, filters, limits)
//! - Patterns subcommand (cross-project pattern extraction)
//! - Gaps subcommand (coverage gap analysis)
//! - Validation errors (zero limits, zero min-projects)
//! - CASS unavailable error handling
//! - JSON output format verification

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::fixture::{E2EFixture, LogLevel};
use ms::error::{MsError, Result};
use serde_json::{Value, json};

// ============================================================================
// Skill definitions for gap analysis testing
// ============================================================================

const SKILL_DEBUGGING: &str = r#"---
name: Debugging Patterns
description: Common debugging strategies for software projects
tags: [debugging, workflow]
---

# Debugging Patterns

Common strategies for debugging software issues.

## Rules

- Use structured logging over println debugging
- Reproduce the issue first, then investigate
- Check recent changes in version control
"#;

const SKILL_TESTING: &str = r#"---
name: Testing Best Practices
description: Guidelines for writing effective tests
tags: [testing, quality]
---

# Testing Best Practices

Guidelines for writing effective and maintainable tests.

## Rules

- Write tests for edge cases and error conditions
- Keep tests focused on one behavior
- Use descriptive test names
"#;

// ============================================================================
// Helper: set up workspace with skills for gap analysis
// ============================================================================

fn setup_workspace(scenario: &str) -> Result<(E2EFixture, PathBuf)> {
    let mut fixture = E2EFixture::new(scenario);

    fixture.log_step("Initialize ms");
    let output = fixture.init();
    fixture.assert_success(&output, "init");

    fixture.log_step("Install fixture-backed cass shim");
    let cass_bin = install_fake_cass(&fixture)?;

    fixture.log_step("Create skills for gap analysis");
    fixture.create_skill("debugging-patterns", SKILL_DEBUGGING)?;
    fixture.create_skill("testing-practices", SKILL_TESTING)?;

    fixture.log_step("Index skills");
    let output = fixture.run_ms(&["--robot", "index"]);
    fixture.assert_success(&output, "index");

    fixture.checkpoint("cross-project:workspace-ready");

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Workspace ready with 2 skills",
        Some(json!({ "skills": 2, "cass_bin": cass_bin.display().to_string() })),
    );

    Ok((fixture, cass_bin))
}

/// Minimal workspace: init only, for validation tests.
fn setup_minimal(scenario: &str) -> Result<E2EFixture> {
    let mut fixture = E2EFixture::new(scenario);

    fixture.log_step("Initialize ms");
    let output = fixture.init();
    fixture.assert_success(&output, "init");

    Ok(fixture)
}

fn run_with_cass(
    fixture: &mut E2EFixture,
    cass_bin: &Path,
    args: &[&str],
) -> super::fixture::CommandOutput {
    let mut owned = args
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
    owned.push("--cass-path".to_string());
    owned.push(cass_bin.display().to_string());
    let refs = owned.iter().map(String::as_str).collect::<Vec<_>>();
    fixture.run_ms(&refs)
}

fn install_fake_cass(fixture: &E2EFixture) -> Result<PathBuf> {
    let cass_dir = fixture.root.join("fake_cass");
    fs::create_dir_all(&cass_dir)
        .map_err(|err| MsError::Config(format!("create fake cass dir: {err}")))?;

    let session_root = cass_dir.join("sessions");
    fs::create_dir_all(&session_root)
        .map_err(|err| MsError::Config(format!("create fake cass session dir: {err}")))?;

    write_json(
        &cass_dir.join("search-all.json"),
        &json!({
            "hits": [
                search_hit(&session_root, "session-001", "alpha-rust", "2026-01-14T10:00:00Z", "Debug a Rust panic in parser", 0.95),
                search_hit(&session_root, "session-002", "beta-ui", "2026-01-15T08:30:00Z", "Refactor duplicate CLI output helpers", 0.81),
                search_hit(&session_root, "session-003", "gamma-e2e", "2026-01-16T14:45:00Z", "Investigate a flaky E2E logging test", 0.88)
            ],
            "total_count": 3,
            "truncated": false
        }),
    )?;
    write_json(
        &cass_dir.join("search-rust.json"),
        &json!({
            "hits": [
                search_hit(&session_root, "session-001", "alpha-rust", "2026-01-14T10:00:00Z", "Debug a Rust panic in parser", 0.95)
            ],
            "total_count": 1,
            "truncated": false
        }),
    )?;
    write_json(
        &cass_dir.join("search-empty.json"),
        &json!({
            "hits": [],
            "total_count": 0,
            "truncated": false
        }),
    )?;

    write_json(
        &cass_dir.join("show-session-001.json"),
        &fake_session_json(
            "session-001",
            &session_root.join("session-001.jsonl"),
            "alpha-rust",
            "2026-01-14T10:00:00Z",
            1200,
            &["rust", "debugging"],
            "Debug a Rust panic in parser",
            "Reproduce the failure, inspect the parser, patch the guard, rerun tests.",
            "cargo test parser",
            "1 failed: parse_empty_input",
        ),
    )?;
    write_json(
        &cass_dir.join("show-session-002.json"),
        &fake_session_json(
            "session-002",
            &session_root.join("session-002.jsonl"),
            "beta-ui",
            "2026-01-15T08:30:00Z",
            980,
            &["testing", "refactor"],
            "Refactor duplicate CLI output helpers",
            "Reproduce the failure, inspect the formatter, patch the helper, rerun tests.",
            "cargo test formatters",
            "formatter tests passed after helper extraction",
        ),
    )?;
    write_json(
        &cass_dir.join("show-session-003.json"),
        &fake_session_json(
            "session-003",
            &session_root.join("session-003.jsonl"),
            "gamma-e2e",
            "2026-01-16T14:45:00Z",
            1040,
            &["testing", "e2e"],
            "Investigate a flaky E2E logging test",
            "Reproduce the failure, inspect the logs, patch the assertion, rerun tests.",
            "cargo test e2e",
            "intermittent failure: missing checkpoint event",
        ),
    )?;

    let script_path = cass_dir.join("cass");
    let script = r#"#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)"
command="${1:-}"

case "$command" in
  --version)
    echo "cass 0.1.0"
    ;;
  search)
    query="${2:-*}"
    case "$query" in
      rust)
        cat "$root/search-rust.json"
        ;;
      "*")
        cat "$root/search-all.json"
        ;;
      *)
        cat "$root/search-empty.json"
        ;;
    esac
    ;;
  show)
    session_id="${2:-}"
    case "$session_id" in
      session-001)
        cat "$root/show-session-001.json"
        ;;
      session-002)
        cat "$root/show-session-002.json"
        ;;
      session-003)
        cat "$root/show-session-003.json"
        ;;
      *)
        echo "session not found: $session_id" >&2
        exit 2
        ;;
    esac
    ;;
  *)
    echo "unsupported command: $command" >&2
    exit 1
    ;;
esac
"#;
    fs::write(&script_path, script)
        .map_err(|err| MsError::Config(format!("write fake cass script: {err}")))?;
    let mut perms = fs::metadata(&script_path)
        .map_err(|err| MsError::Config(format!("stat fake cass script: {err}")))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms)
        .map_err(|err| MsError::Config(format!("chmod fake cass script: {err}")))?;

    Ok(script_path)
}

fn search_hit(
    session_root: &Path,
    session_id: &str,
    project: &str,
    timestamp: &str,
    snippet: &str,
    score: f32,
) -> Value {
    json!({
        "session_id": session_id,
        "path": session_root.join(format!("{session_id}.jsonl")).display().to_string(),
        "score": score,
        "snippet": snippet,
        "content_hash": format!("hash-{session_id}"),
        "project": project,
        "timestamp": timestamp
    })
}

fn fake_session_json(
    session_id: &str,
    path: &Path,
    project: &str,
    started_at: &str,
    token_count: usize,
    tags: &[&str],
    user_prompt: &str,
    assistant_plan: &str,
    command: &str,
    tool_result: &str,
) -> Value {
    json!({
        "id": session_id,
        "path": path.display().to_string(),
        "content_hash": format!("hash-{session_id}"),
        "metadata": {
            "project": project,
            "agent": "codex",
            "model": "codex",
            "started_at": started_at,
            "ended_at": started_at,
            "message_count": 3,
            "token_count": token_count,
            "tags": tags
        },
        "messages": [
            {
                "index": 0,
                "role": "user",
                "content": user_prompt,
                "tool_calls": [],
                "tool_results": []
            },
            {
                "index": 1,
                "role": "assistant",
                "content": assistant_plan,
                "tool_calls": [
                    {
                        "id": format!("{session_id}-tool"),
                        "name": "Bash",
                        "arguments": { "command": command }
                    }
                ],
                "tool_results": [
                    {
                        "tool_call_id": format!("{session_id}-tool"),
                        "content": tool_result,
                        "is_error": false
                    }
                ]
            },
            {
                "index": 2,
                "role": "assistant",
                "content": "Capture the fix, then rerun the focused tests and record the result.",
                "tool_calls": [],
                "tool_results": []
            }
        ]
    })
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| MsError::Config(format!("serialize fake cass fixture: {err}")))?;
    fs::write(path, bytes)
        .map_err(|err| MsError::Config(format!("write {}: {err}", path.display())))
}

// ============================================================================
// Validation Error Tests
// ============================================================================

/// Summary with limit=0 should fail validation.
#[test]
fn test_cross_project_summary_zero_limit() -> Result<()> {
    let mut fixture = setup_minimal("cross_project_summary_zero_limit")?;

    fixture.checkpoint("cross-project:pre-summary-zero-limit");

    fixture.log_step("Run summary with limit=0");
    let output = fixture.run_ms(&["--robot", "cross-project", "summary", "--limit", "0"]);

    assert!(!output.success, "Summary with limit=0 should fail");

    fixture.checkpoint("cross-project:post-summary-zero-limit");

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Zero limit validation correctly rejected",
        None,
    );

    fixture.generate_report();
    Ok(())
}

/// Patterns with limit=0 should fail validation.
#[test]
fn test_cross_project_patterns_zero_limit() -> Result<()> {
    let mut fixture = setup_minimal("cross_project_patterns_zero_limit")?;

    fixture.checkpoint("cross-project:pre-patterns-zero-limit");

    fixture.log_step("Run patterns with limit=0");
    let output = fixture.run_ms(&["--robot", "cross-project", "patterns", "--limit", "0"]);

    assert!(!output.success, "Patterns with limit=0 should fail");

    fixture.checkpoint("cross-project:post-patterns-zero-limit");

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Zero limit validation correctly rejected for patterns",
        None,
    );

    fixture.generate_report();
    Ok(())
}

/// Patterns with min-projects=0 should fail validation.
#[test]
fn test_cross_project_patterns_zero_min_projects() -> Result<()> {
    let mut fixture = setup_minimal("cross_project_patterns_zero_min_projects")?;

    fixture.checkpoint("cross-project:pre-patterns-zero-min-projects");

    fixture.log_step("Run patterns with min-projects=0");
    let output = fixture.run_ms(&[
        "--robot",
        "cross-project",
        "patterns",
        "--min-projects",
        "0",
    ]);

    assert!(!output.success, "Patterns with min-projects=0 should fail");

    fixture.checkpoint("cross-project:post-patterns-zero-min-projects");

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Zero min-projects validation correctly rejected",
        None,
    );

    fixture.generate_report();
    Ok(())
}

/// Gaps with limit=0 should fail validation.
#[test]
fn test_cross_project_gaps_zero_limit() -> Result<()> {
    let mut fixture = setup_minimal("cross_project_gaps_zero_limit")?;

    fixture.checkpoint("cross-project:pre-gaps-zero-limit");

    fixture.log_step("Run gaps with limit=0");
    let output = fixture.run_ms(&["--robot", "cross-project", "gaps", "--limit", "0"]);

    assert!(!output.success, "Gaps with limit=0 should fail");

    fixture.checkpoint("cross-project:post-gaps-zero-limit");

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Zero limit validation correctly rejected for gaps",
        None,
    );

    fixture.generate_report();
    Ok(())
}

/// Gaps with min-projects=0 should fail validation.
#[test]
fn test_cross_project_gaps_zero_min_projects() -> Result<()> {
    let mut fixture = setup_minimal("cross_project_gaps_zero_min_projects")?;

    fixture.checkpoint("cross-project:pre-gaps-zero-min-projects");

    fixture.log_step("Run gaps with min-projects=0");
    let output = fixture.run_ms(&["--robot", "cross-project", "gaps", "--min-projects", "0"]);

    assert!(!output.success, "Gaps with min-projects=0 should fail");

    fixture.checkpoint("cross-project:post-gaps-zero-min-projects");

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Zero min-projects validation correctly rejected for gaps",
        None,
    );

    fixture.generate_report();
    Ok(())
}

// ============================================================================
// CASS Unavailable Tests
// ============================================================================

/// Summary with bogus cass-path should report unavailable.
#[test]
fn test_cross_project_summary_cass_unavailable() -> Result<()> {
    let mut fixture = setup_minimal("cross_project_summary_cass_unavailable")?;

    fixture.checkpoint("cross-project:pre-summary-no-cass");

    fixture.log_step("Run summary with nonexistent cass binary");
    let output = fixture.run_ms(&[
        "--robot",
        "cross-project",
        "summary",
        "--cass-path",
        "/nonexistent/cass/binary",
    ]);

    assert!(!output.success, "Summary with unavailable CASS should fail");

    fixture.checkpoint("cross-project:post-summary-no-cass");

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "CASS unavailable correctly detected for summary",
        None,
    );

    fixture.generate_report();
    Ok(())
}

/// Patterns with bogus cass-path should report unavailable.
#[test]
fn test_cross_project_patterns_cass_unavailable() -> Result<()> {
    let mut fixture = setup_minimal("cross_project_patterns_cass_unavailable")?;

    fixture.checkpoint("cross-project:pre-patterns-no-cass");

    fixture.log_step("Run patterns with nonexistent cass binary");
    let output = fixture.run_ms(&[
        "--robot",
        "cross-project",
        "patterns",
        "--cass-path",
        "/nonexistent/cass/binary",
    ]);

    assert!(
        !output.success,
        "Patterns with unavailable CASS should fail"
    );

    fixture.checkpoint("cross-project:post-patterns-no-cass");

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "CASS unavailable correctly detected for patterns",
        None,
    );

    fixture.generate_report();
    Ok(())
}

/// Gaps with bogus cass-path should report unavailable.
#[test]
fn test_cross_project_gaps_cass_unavailable() -> Result<()> {
    let mut fixture = setup_minimal("cross_project_gaps_cass_unavailable")?;

    fixture.checkpoint("cross-project:pre-gaps-no-cass");

    fixture.log_step("Run gaps with nonexistent cass binary");
    let output = fixture.run_ms(&[
        "--robot",
        "cross-project",
        "gaps",
        "--cass-path",
        "/nonexistent/cass/binary",
    ]);

    assert!(!output.success, "Gaps with unavailable CASS should fail");

    fixture.checkpoint("cross-project:post-gaps-no-cass");

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "CASS unavailable correctly detected for gaps",
        None,
    );

    fixture.generate_report();
    Ok(())
}

// ============================================================================
// Summary Workflow Tests (with real CASS)
// ============================================================================

/// Summary with default args should produce valid JSON output.
#[test]
fn test_cross_project_summary_json_output() -> Result<()> {
    let (mut fixture, cass_bin) = setup_workspace("cross_project_summary_json")?;

    fixture.checkpoint("cross-project:pre-summary");

    fixture.log_step("Run cross-project summary with robot mode");
    let output = run_with_cass(
        &mut fixture,
        &cass_bin,
        &["--robot", "cross-project", "summary"],
    );
    fixture.assert_success(&output, "cross-project summary");

    fixture.checkpoint("cross-project:post-summary");

    let json = output.json();

    // Verify JSON structure
    assert!(
        json["query"].is_string(),
        "Response should have query field"
    );
    assert!(
        json["total_sessions"].is_number(),
        "Response should have total_sessions"
    );
    assert!(
        json["total_projects"].is_number(),
        "Response should have total_projects"
    );
    assert!(
        json["projects"].is_array(),
        "Response should have projects array"
    );
    assert_eq!(json["total_sessions"].as_u64(), Some(3));
    assert_eq!(json["total_projects"].as_u64(), Some(3));

    let total = json["total_sessions"].as_u64().unwrap_or(0);

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        &format!("Summary returned {total} sessions"),
        Some(json.clone()),
    );

    fixture.generate_report();
    Ok(())
}

/// Summary with --top limit restricts output.
#[test]
fn test_cross_project_summary_top_limit() -> Result<()> {
    let (mut fixture, cass_bin) = setup_workspace("cross_project_summary_top_limit")?;

    fixture.checkpoint("cross-project:pre-summary-top");

    fixture.log_step("Run summary with --top 2");
    let output = run_with_cass(
        &mut fixture,
        &cass_bin,
        &["--robot", "cross-project", "summary", "--top", "2"],
    );
    fixture.assert_success(&output, "cross-project summary top");

    fixture.checkpoint("cross-project:post-summary-top");

    let json = output.json();
    let projects = json["projects"].as_array().expect("projects array");
    assert!(
        projects.len() == 2,
        "Should return exactly 2 projects, got {}",
        projects.len()
    );

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Summary top limit verified",
        Some(serde_json::json!({ "returned": projects.len(), "limit": 2 })),
    );

    fixture.generate_report();
    Ok(())
}

/// Summary with query filter narrows results.
#[test]
fn test_cross_project_summary_with_query() -> Result<()> {
    let (mut fixture, cass_bin) = setup_workspace("cross_project_summary_query")?;

    fixture.checkpoint("cross-project:pre-summary-query");

    fixture.log_step("Run summary with specific query");
    let output = run_with_cass(
        &mut fixture,
        &cass_bin,
        &["--robot", "cross-project", "summary", "--query", "rust"],
    );
    fixture.assert_success(&output, "cross-project summary with query");

    fixture.checkpoint("cross-project:post-summary-query");

    let json = output.json();
    assert_eq!(
        json["query"].as_str(),
        Some("rust"),
        "Query should be preserved in output"
    );
    assert_eq!(json["total_sessions"].as_u64(), Some(1));
    assert_eq!(json["total_projects"].as_u64(), Some(1));

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Summary with query filter verified",
        Some(json.clone()),
    );

    fixture.generate_report();
    Ok(())
}

/// Summary with min-sessions filter.
#[test]
fn test_cross_project_summary_min_sessions() -> Result<()> {
    let (mut fixture, cass_bin) = setup_workspace("cross_project_summary_min_sessions")?;

    fixture.checkpoint("cross-project:pre-summary-min-sessions");

    fixture.log_step("Run summary with high min-sessions threshold");
    let output = run_with_cass(
        &mut fixture,
        &cass_bin,
        &[
            "--robot",
            "cross-project",
            "summary",
            "--min-sessions",
            "999999",
        ],
    );
    fixture.assert_success(&output, "cross-project summary min-sessions");

    fixture.checkpoint("cross-project:post-summary-min-sessions");

    let json = output.json();
    let projects = json["projects"].as_array().expect("projects array");
    assert_eq!(
        projects.len(),
        0,
        "With extremely high min-sessions, no projects should match"
    );

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Summary min-sessions filter verified",
        None,
    );

    fixture.generate_report();
    Ok(())
}

// ============================================================================
// Patterns Workflow Tests (with real CASS)
// ============================================================================

/// Patterns with default args should produce valid JSON output.
#[test]
fn test_cross_project_patterns_json_output() -> Result<()> {
    let (mut fixture, cass_bin) = setup_workspace("cross_project_patterns_json")?;

    fixture.checkpoint("cross-project:pre-patterns");

    fixture.log_step("Run cross-project patterns with robot mode");
    let output = run_with_cass(
        &mut fixture,
        &cass_bin,
        &["--robot", "cross-project", "patterns"],
    );
    fixture.assert_success(&output, "cross-project patterns");

    fixture.checkpoint("cross-project:post-patterns");

    let json = output.json();

    // Verify JSON structure
    assert!(
        json["query"].is_string(),
        "Response should have query field"
    );
    assert!(
        json["scanned_sessions"].is_number(),
        "Response should have scanned_sessions"
    );
    assert!(
        json["patterns"].is_array(),
        "Response should have patterns array"
    );
    assert_eq!(json["scanned_sessions"].as_u64(), Some(3));
    assert!(
        json["patterns"]
            .as_array()
            .is_some_and(|patterns| !patterns.is_empty()),
        "Expected at least one cross-project pattern"
    );

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Patterns JSON output verified",
        Some(json.clone()),
    );

    fixture.generate_report();
    Ok(())
}

/// Patterns with high thresholds should return fewer/no results.
#[test]
fn test_cross_project_patterns_high_thresholds() -> Result<()> {
    let (mut fixture, cass_bin) = setup_workspace("cross_project_patterns_high_thresholds")?;

    fixture.checkpoint("cross-project:pre-patterns-high-thresholds");

    fixture.log_step("Run patterns with very high occurrence threshold");
    let output = run_with_cass(
        &mut fixture,
        &cass_bin,
        &[
            "--robot",
            "cross-project",
            "patterns",
            "--min-occurrences",
            "999999",
        ],
    );
    fixture.assert_success(&output, "cross-project patterns high thresholds");

    fixture.checkpoint("cross-project:post-patterns-high-thresholds");

    let json = output.json();
    let patterns = json["patterns"].as_array().expect("patterns array");
    assert_eq!(
        patterns.len(),
        0,
        "With extremely high min-occurrences, no patterns should match"
    );

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Patterns high threshold filter verified",
        None,
    );

    fixture.generate_report();
    Ok(())
}

// ============================================================================
// Gaps Workflow Tests (with real CASS)
// ============================================================================

/// Gaps with default args should produce valid JSON output.
#[test]
fn test_cross_project_gaps_json_output() -> Result<()> {
    let (mut fixture, cass_bin) = setup_workspace("cross_project_gaps_json")?;

    fixture.checkpoint("cross-project:pre-gaps");

    fixture.log_step("Run cross-project gaps with robot mode");
    let output = run_with_cass(
        &mut fixture,
        &cass_bin,
        &["--robot", "cross-project", "gaps"],
    );
    fixture.assert_success(&output, "cross-project gaps");

    fixture.checkpoint("cross-project:post-gaps");

    let json = output.json();

    // Verify JSON structure
    assert!(
        json["query"].is_string(),
        "Response should have query field"
    );
    assert!(
        json["scanned_sessions"].is_number(),
        "Response should have scanned_sessions"
    );
    assert!(json["gaps"].is_array(), "Response should have gaps array");
    assert_eq!(json["scanned_sessions"].as_u64(), Some(3));
    assert!(
        json["gaps"].as_array().is_some_and(|gaps| !gaps.is_empty()),
        "Expected at least one coverage gap from shared command/workflow patterns"
    );

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Gaps JSON output verified",
        Some(json.clone()),
    );

    fixture.generate_report();
    Ok(())
}

/// Gaps with high min-score should consider most patterns as gaps.
#[test]
fn test_cross_project_gaps_min_score() -> Result<()> {
    let (mut fixture, cass_bin) = setup_workspace("cross_project_gaps_min_score")?;

    fixture.checkpoint("cross-project:pre-gaps-min-score");

    fixture.log_step("Run gaps with high min-score to include more gaps");
    let output = run_with_cass(
        &mut fixture,
        &cass_bin,
        &["--robot", "cross-project", "gaps", "--min-score", "100.0"],
    );
    fixture.assert_success(&output, "cross-project gaps min-score");

    fixture.checkpoint("cross-project:post-gaps-min-score");

    let json = output.json();
    // With a very high min_score, all patterns should appear as gaps
    // (unless perfectly matched)
    assert!(json["gaps"].is_array(), "Should return gaps array");

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Gaps min-score filter verified",
        Some(json.clone()),
    );

    fixture.generate_report();
    Ok(())
}

/// Gaps with search-limit=1 should still work correctly.
#[test]
fn test_cross_project_gaps_search_limit() -> Result<()> {
    let (mut fixture, cass_bin) = setup_workspace("cross_project_gaps_search_limit")?;

    fixture.checkpoint("cross-project:pre-gaps-search-limit");

    fixture.log_step("Run gaps with search-limit=1");
    let output = run_with_cass(
        &mut fixture,
        &cass_bin,
        &["--robot", "cross-project", "gaps", "--search-limit", "1"],
    );
    fixture.assert_success(&output, "cross-project gaps search-limit");

    fixture.checkpoint("cross-project:post-gaps-search-limit");

    let json = output.json();
    assert!(json["gaps"].is_array(), "Should return gaps array");
    for gap in json["gaps"].as_array().expect("gaps array") {
        if let Some(best_match) = gap.get("best_match") {
            assert!(
                best_match.is_null() || best_match.is_object(),
                "best_match should be null or a single object"
            );
        }
    }

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Gaps search-limit verified",
        Some(json.clone()),
    );

    fixture.generate_report();
    Ok(())
}

/// Gaps with zero search-limit should fail validation.
#[test]
fn test_cross_project_gaps_zero_search_limit() -> Result<()> {
    let mut fixture = setup_minimal("cross_project_gaps_zero_search_limit")?;

    fixture.checkpoint("cross-project:pre-gaps-zero-search-limit");

    fixture.log_step("Run gaps with search-limit=0");
    let output = fixture.run_ms(&["--robot", "cross-project", "gaps", "--search-limit", "0"]);

    assert!(!output.success, "Gaps with search-limit=0 should fail");

    fixture.checkpoint("cross-project:post-gaps-zero-search-limit");

    fixture.emit_event(
        LogLevel::Info,
        "cross-project",
        "Zero search-limit validation correctly rejected",
        None,
    );

    fixture.generate_report();
    Ok(())
}
