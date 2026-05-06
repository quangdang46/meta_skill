//! E2E Scenario: Sync Workflow
//!
//! Exercises the filesystem sync flow with real `ms` CLI calls:
//! - Initialize sync against a real archive root
//! - Push skills to a fresh remote
//! - Pull skills into a second machine
//! - Detect and resolve conflicts
//! - Verify dry-run behavior
//! - Verify status reporting

use std::fs;
use std::path::{Path, PathBuf};

use super::fixture::{CommandOutput, E2EFixture};
use ms::error::{MsError, Result};
use ms::sync::{ConflictStrategy, SyncConfig};
use serde_json::Value;

fn build_skill(name: &str, description: &str, tags: &[&str], body: &str) -> String {
    format!(
        r#"---
name: {name}
description: {description}
tags: [{tags}]
---

# {name}

{body}
"#,
        tags = tags.join(", "),
    )
}

fn setup_fixture(scenario: &str) -> Result<E2EFixture> {
    let mut fixture = E2EFixture::new(scenario);

    fixture.log_step("Initialize ms");
    let output = fixture.init();
    fixture.assert_success(&output, "init");
    fixture.configure_default_skill_paths();
    disable_auto_index(&fixture)?;
    fixture.checkpoint("sync:init");

    Ok(fixture)
}

fn disable_auto_index(fixture: &E2EFixture) -> Result<()> {
    let raw = fs::read_to_string(&fixture.config_path)
        .map_err(|err| MsError::Config(format!("read fixture config for sync tests: {err}")))?;
    let mut doc = toml::from_str::<toml::Value>(&raw)
        .map_err(|err| MsError::Config(format!("parse fixture config for sync tests: {err}")))?;

    let root = doc
        .as_table_mut()
        .ok_or_else(|| MsError::Config("fixture config should be a TOML table".to_string()))?;
    let ru = root
        .entry("ru".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let ru = ru
        .as_table_mut()
        .ok_or_else(|| MsError::Config("fixture ru config should be a TOML table".to_string()))?;
    ru.insert("auto_index".to_string(), toml::Value::Boolean(false));

    let rendered = toml::to_string_pretty(&doc)
        .map_err(|err| MsError::Config(format!("render fixture sync config: {err}")))?;
    fs::write(&fixture.config_path, rendered)
        .map_err(|err| MsError::Config(format!("write fixture sync config: {err}")))
}

fn seed_skill(
    fixture: &mut E2EFixture,
    skill_id: &str,
    content: &str,
    checkpoint: &str,
) -> Result<()> {
    fixture.log_step(&format!("Create skill {skill_id}"));
    fixture.create_skill(skill_id, content)?;

    fixture.log_step(&format!("Index skill {skill_id}"));
    let output = fixture.run_ms(&["--robot", "index"]);
    fixture.assert_success(&output, "index");
    fixture.checkpoint(checkpoint);

    Ok(())
}

fn add_filesystem_remote(
    fixture: &mut E2EFixture,
    name: &str,
    remote_ms_root: &Path,
    checkpoint: &str,
) -> Result<()> {
    let remote_url = remote_ms_root.to_string_lossy().into_owned();

    fixture.log_step(&format!("Add remote {name}"));
    let output = fixture.run_ms(&[
        "--robot",
        "remote",
        "add",
        name,
        remote_url.as_str(),
        "--remote-type",
        "filesystem",
    ]);
    fixture.assert_success(&output, "remote add");
    fixture.checkpoint(checkpoint);

    Ok(())
}

fn sync_report(output: &CommandOutput) -> Result<Value> {
    let json = output.json();
    let reports = json["reports"].as_array().ok_or_else(|| {
        MsError::ValidationFailed("sync output should contain a reports array".to_string())
    })?;

    reports.first().cloned().ok_or_else(|| {
        MsError::ValidationFailed("sync output should include one report".to_string())
    })
}

fn run_sync_report(
    fixture: &mut E2EFixture,
    args: &[&str],
    operation: &str,
    checkpoint: &str,
) -> Result<Value> {
    fixture.log_step(operation);
    let output = fixture.run_ms(args);
    fixture.assert_success(&output, operation);
    fixture.checkpoint(checkpoint);
    sync_report(&output)
}

fn sync_status_json(fixture: &mut E2EFixture, checkpoint: &str) -> Result<Value> {
    fixture.log_step("Check sync status");
    let output = fixture.run_ms(&["--robot", "sync", "--status"]);
    fixture.assert_success(&output, "sync status");
    fixture.checkpoint(checkpoint);
    Ok(output.json())
}

fn load_skill_description(fixture: &mut E2EFixture, skill_id: &str) -> Result<String> {
    fixture.log_step(&format!("Load skill {skill_id}"));
    let output = fixture.run_ms(&["--robot", "load", skill_id, "--full"]);
    fixture.assert_success(&output, "load synced skill");

    output.json()["data"]["frontmatter"]["description"]
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            MsError::ValidationFailed(
                "load output missing data.frontmatter.description".to_string(),
            )
        })
}

fn set_conflict_strategy(
    fixture: &E2EFixture,
    skill_id: &str,
    strategy: ConflictStrategy,
) -> Result<()> {
    let path = fixture.root.join(".config/ms/sync.toml");
    let mut config = SyncConfig::load_from(&path)?;
    config
        .conflict_strategies
        .insert(skill_id.to_string(), strategy);
    config.save_to(&path)
}

fn archive_root(fixture: &E2EFixture) -> PathBuf {
    fixture.ms_root.join("archive")
}

fn archived_skill_dir(archive_root: &Path, skill_id: &str) -> PathBuf {
    skill_id
        .split('/')
        .fold(archive_root.join("skills/by-id"), |path, segment| {
            path.join(segment)
        })
}

fn archived_skill_description(archive_root: &Path, skill_id: &str) -> Result<String> {
    let path = archived_skill_dir(archive_root, skill_id).join("skill.spec.json");
    let raw = fs::read_to_string(&path)
        .map_err(|err| MsError::Config(format!("read archived skill {}: {err}", path.display())))?;
    let json = serde_json::from_str::<Value>(&raw).map_err(|err| {
        MsError::Config(format!(
            "parse archived skill spec {}: {err}",
            path.display()
        ))
    })?;
    json["metadata"]["description"]
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            MsError::ValidationFailed(format!(
                "archived skill spec missing metadata.description at {}",
                path.display()
            ))
        })
}

fn array_contains(value: &Value, expected: &str) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(expected)))
}

#[test]
fn test_sync_fresh_remote() -> Result<()> {
    let mut local = setup_fixture("sync_fresh_remote_local")?;
    let remote = setup_fixture("sync_fresh_remote_remote")?;
    let skill_id = "sync-test-skill";

    seed_skill(
        &mut local,
        skill_id,
        &build_skill(
            "Sync Test Skill",
            "A skill to test fresh remote sync",
            &["test", "sync"],
            "Fresh remote sync content.",
        ),
        "sync:local_indexed",
    )?;
    add_filesystem_remote(
        &mut local,
        "fresh-remote",
        &remote.ms_root,
        "sync:remote_added",
    )?;

    let report = run_sync_report(
        &mut local,
        &["--robot", "sync", "fresh-remote"],
        "Sync to fresh remote",
        "sync:post_sync",
    )?;

    assert!(array_contains(&report["pushed"], skill_id));
    assert_eq!(report["conflicts"].as_array().map_or(0, Vec::len), 0);
    assert!(
        archived_skill_dir(&archive_root(&remote), skill_id)
            .join("skill.spec.json")
            .exists(),
        "expected pushed skill to exist in remote archive"
    );

    let status = sync_status_json(&mut local, "sync:status")?;
    assert!(status["last_full_sync"].get("fresh-remote").is_some());
    assert_eq!(status["status_counts"]["Synced"].as_u64(), Some(1));

    Ok(())
}

#[test]
fn test_sync_pull_changes() -> Result<()> {
    let mut source = setup_fixture("sync_pull_changes_source")?;
    let remote = setup_fixture("sync_pull_changes_remote")?;
    let mut target = setup_fixture("sync_pull_changes_target")?;
    let skill_id = "shared-skill";

    seed_skill(
        &mut source,
        skill_id,
        &build_skill(
            "Shared Skill",
            "Shared remote content for the pull workflow",
            &["test", "shared"],
            "Shared remote content for the pull workflow.",
        ),
        "sync:source_indexed",
    )?;
    add_filesystem_remote(
        &mut source,
        "shared-remote",
        &remote.ms_root,
        "sync:source_remote",
    )?;

    let push_report = run_sync_report(
        &mut source,
        &["--robot", "sync", "shared-remote", "--push-only"],
        "Push source skill to shared remote",
        "sync:source_pushed",
    )?;
    assert!(array_contains(&push_report["pushed"], skill_id));

    add_filesystem_remote(
        &mut target,
        "shared-remote",
        &remote.ms_root,
        "sync:target_remote",
    )?;
    let pull_report = run_sync_report(
        &mut target,
        &["--robot", "sync", "shared-remote", "--pull-only"],
        "Pull changes into second machine",
        "sync:target_pulled",
    )?;
    assert!(array_contains(&pull_report["pulled"], skill_id));

    let description = load_skill_description(&mut target, skill_id)?;
    assert_eq!(description, "Shared remote content for the pull workflow");

    Ok(())
}

#[test]
fn test_sync_conflict_detection() -> Result<()> {
    let mut local = setup_fixture("sync_conflict_detection_local")?;
    let mut remote = setup_fixture("sync_conflict_detection_remote")?;
    let skill_id = "conflict-skill";

    seed_skill(
        &mut remote,
        skill_id,
        &build_skill(
            "Conflict Skill",
            "Base version before divergence",
            &["sync", "conflict"],
            "Base content before either machine changes the skill.",
        ),
        "sync:remote_seeded",
    )?;
    add_filesystem_remote(
        &mut local,
        "conflict-remote",
        &remote.ms_root,
        "sync:remote_registered",
    )?;

    let initial_pull = run_sync_report(
        &mut local,
        &["--robot", "sync", "conflict-remote", "--pull-only"],
        "Pull base skill before divergence",
        "sync:base_pulled",
    )?;
    assert!(array_contains(&initial_pull["pulled"], skill_id));

    seed_skill(
        &mut local,
        skill_id,
        &build_skill(
            "Conflict Skill",
            "Local conflict variant",
            &["sync", "conflict"],
            "Local variant body that should remain until conflict resolution.",
        ),
        "sync:local_updated",
    )?;
    seed_skill(
        &mut remote,
        skill_id,
        &build_skill(
            "Conflict Skill",
            "Remote conflict variant",
            &["sync", "conflict"],
            "Remote variant body that should trigger a conflict.",
        ),
        "sync:remote_updated",
    )?;

    let conflict_report = run_sync_report(
        &mut local,
        &["--robot", "sync", "conflict-remote"],
        "Run sync without force to surface conflict",
        "sync:conflict_detected",
    )?;
    assert!(array_contains(&conflict_report["conflicts"], skill_id));
    assert_eq!(
        conflict_report["resolved"].as_array().map_or(0, Vec::len),
        0
    );

    let status = sync_status_json(&mut local, "sync:conflict_status")?;
    assert_eq!(status["status_counts"]["Conflict"].as_u64(), Some(1));

    let local_description = load_skill_description(&mut local, skill_id)?;
    assert_eq!(local_description, "Local conflict variant");

    let remote_description = archived_skill_description(&archive_root(&remote), skill_id)?;
    assert_eq!(remote_description, "Remote conflict variant");

    Ok(())
}

#[test]
fn test_sync_conflict_resolution() -> Result<()> {
    let mut local = setup_fixture("sync_conflict_resolution_local")?;
    let mut remote = setup_fixture("sync_conflict_resolution_remote")?;
    let skill_id = "resolution-skill";

    seed_skill(
        &mut remote,
        skill_id,
        &build_skill(
            "Resolution Skill",
            "Base version before forced resolution",
            &["sync", "resolution"],
            "Base content for conflict resolution.",
        ),
        "sync:remote_seeded",
    )?;
    add_filesystem_remote(
        &mut local,
        "resolution-remote",
        &remote.ms_root,
        "sync:remote_registered",
    )?;

    let base_pull = run_sync_report(
        &mut local,
        &["--robot", "sync", "resolution-remote", "--pull-only"],
        "Pull base skill before resolution test",
        "sync:base_pulled",
    )?;
    assert!(array_contains(&base_pull["pulled"], skill_id));

    seed_skill(
        &mut local,
        skill_id,
        &build_skill(
            "Resolution Skill",
            "Local resolution variant",
            &["sync", "resolution"],
            "Local resolution body that should be replaced by the remote version.",
        ),
        "sync:local_updated",
    )?;
    seed_skill(
        &mut remote,
        skill_id,
        &build_skill(
            "Resolution Skill",
            "Remote resolution variant",
            &["sync", "resolution"],
            "Remote resolution body that should win after forced sync.",
        ),
        "sync:remote_updated",
    )?;
    set_conflict_strategy(&local, skill_id, ConflictStrategy::PreferRemote)?;

    let report = run_sync_report(
        &mut local,
        &["--robot", "sync", "resolution-remote", "--force"],
        "Resolve conflict with prefer-remote strategy",
        "sync:conflict_resolved",
    )?;
    assert!(array_contains(&report["resolved"], skill_id));
    assert!(array_contains(&report["pulled"], skill_id));
    assert_eq!(report["conflicts"].as_array().map_or(0, Vec::len), 0);

    let description = load_skill_description(&mut local, skill_id)?;
    assert_eq!(description, "Remote resolution variant");

    let status = sync_status_json(&mut local, "sync:resolved_status")?;
    assert_eq!(status["status_counts"]["Synced"].as_u64(), Some(1));

    Ok(())
}

#[test]
fn test_sync_dry_run() -> Result<()> {
    let mut local = setup_fixture("sync_dry_run_local")?;
    let remote = setup_fixture("sync_dry_run_remote")?;
    let skill_id = "dry-run-skill";

    seed_skill(
        &mut local,
        skill_id,
        &build_skill(
            "Dry Run Skill",
            "Testing dry-run sync",
            &["test", "sync"],
            "Dry-run content that should never be written to the remote archive.",
        ),
        "sync:local_indexed",
    )?;
    add_filesystem_remote(
        &mut local,
        "dry-remote",
        &remote.ms_root,
        "sync:remote_added",
    )?;

    let files_before = walkdir::WalkDir::new(archive_root(&remote))
        .into_iter()
        .filter_map(std::result::Result::ok)
        .count();

    let report = run_sync_report(
        &mut local,
        &["--robot", "sync", "dry-remote", "--dry-run"],
        "Preview sync with dry-run",
        "sync:dry_run_complete",
    )?;
    assert!(array_contains(&report["pushed"], skill_id));
    assert!(
        !archived_skill_dir(&archive_root(&remote), skill_id)
            .join("skill.spec.json")
            .exists(),
        "dry-run should not write the skill to the remote archive"
    );

    let files_after = walkdir::WalkDir::new(archive_root(&remote))
        .into_iter()
        .filter_map(std::result::Result::ok)
        .count();
    assert_eq!(
        files_before, files_after,
        "dry-run should leave remote files unchanged"
    );

    let status = sync_status_json(&mut local, "sync:dry_run_status")?;
    assert!(status["last_full_sync"].get("dry-remote").is_none());

    Ok(())
}

#[test]
fn test_sync_status_check() -> Result<()> {
    let mut local = setup_fixture("sync_status_check_local")?;
    let remote = setup_fixture("sync_status_check_remote")?;
    let skill_id = "status-skill";
    let remote_url = remote.ms_root.to_string_lossy().into_owned();

    seed_skill(
        &mut local,
        skill_id,
        &build_skill(
            "Status Skill",
            "Skill used to verify sync status reporting",
            &["sync", "status"],
            "Status reporting content.",
        ),
        "sync:local_indexed",
    )?;
    add_filesystem_remote(
        &mut local,
        "status-remote",
        &remote.ms_root,
        "sync:remote_added",
    )?;

    let report = run_sync_report(
        &mut local,
        &["--robot", "sync", "status-remote"],
        "Sync skill before status check",
        "sync:post_sync",
    )?;
    assert!(array_contains(&report["pushed"], skill_id));

    let status = sync_status_json(&mut local, "sync:post_status")?;
    assert_eq!(status["status"].as_str(), Some("ok"));
    assert!(status["last_full_sync"].get("status-remote").is_some());
    assert_eq!(status["status_counts"]["Synced"].as_u64(), Some(1));
    assert!(
        status["remotes"]
            .as_array()
            .is_some_and(|remotes| remotes.iter().any(|remote_entry| {
                remote_entry["name"].as_str() == Some("status-remote")
                    && remote_entry["url"].as_str() == Some(remote_url.as_str())
            })),
        "status output should include the configured remote"
    );

    Ok(())
}

#[test]
fn test_remote_management() -> Result<()> {
    let mut fixture = setup_fixture("remote_management")?;
    let remote = setup_fixture("remote_management_remote")?;
    let remote_url = remote.ms_root.to_string_lossy().into_owned();

    fixture.log_step("Add remote");
    let output = fixture.run_ms(&[
        "--robot",
        "remote",
        "add",
        "managed-remote",
        remote_url.as_str(),
        "--remote-type",
        "filesystem",
    ]);
    fixture.assert_success(&output, "remote add");
    fixture.checkpoint("sync:remote_added");

    fixture.log_step("List remotes");
    let output = fixture.run_ms(&["--robot", "remote", "list"]);
    fixture.assert_success(&output, "remote list");
    let list = output.json();
    assert!(
        list["remotes"]
            .as_array()
            .is_some_and(|remotes| remotes.iter().any(|remote| {
                remote["name"].as_str() == Some("managed-remote")
                    && remote["enabled"].as_bool() == Some(true)
            })),
        "remote list should include the enabled remote"
    );

    fixture.log_step("Disable remote");
    let output = fixture.run_ms(&["--robot", "remote", "disable", "managed-remote"]);
    fixture.assert_success(&output, "remote disable");
    assert_eq!(output.json()["enabled"].as_bool(), Some(false));

    fixture.log_step("Enable remote");
    let output = fixture.run_ms(&["--robot", "remote", "enable", "managed-remote"]);
    fixture.assert_success(&output, "remote enable");
    assert_eq!(output.json()["enabled"].as_bool(), Some(true));

    fixture.log_step("Remove remote");
    let output = fixture.run_ms(&["--robot", "remote", "remove", "managed-remote"]);
    fixture.assert_success(&output, "remote remove");
    assert_eq!(output.json()["removed"].as_str(), Some("managed-remote"));

    fixture.log_step("Verify remote removal");
    let output = fixture.run_ms(&["--robot", "remote", "list"]);
    fixture.assert_success(&output, "remote list after remove");
    let list = output.json();
    assert!(
        list["remotes"].as_array().is_some_and(|remotes| remotes
            .iter()
            .all(|remote| { remote["name"].as_str() != Some("managed-remote") })),
        "remote list should not contain removed remotes"
    );

    Ok(())
}

#[test]
fn test_sync_push_only() -> Result<()> {
    let mut local = setup_fixture("sync_push_only_local")?;
    let remote = setup_fixture("sync_push_only_remote")?;
    let skill_id = "push-only-skill";

    seed_skill(
        &mut local,
        skill_id,
        &build_skill(
            "Push Only Skill",
            "Testing push-only sync",
            &["sync", "push"],
            "Push-only content written from the local machine.",
        ),
        "sync:local_indexed",
    )?;
    add_filesystem_remote(
        &mut local,
        "push-remote",
        &remote.ms_root,
        "sync:remote_added",
    )?;

    let report = run_sync_report(
        &mut local,
        &["--robot", "sync", "push-remote", "--push-only"],
        "Sync with push-only mode",
        "sync:push_only_complete",
    )?;
    assert!(array_contains(&report["pushed"], skill_id));
    assert_eq!(report["pulled"].as_array().map_or(0, Vec::len), 0);
    assert!(
        archived_skill_dir(&archive_root(&remote), skill_id)
            .join("skill.spec.json")
            .exists(),
        "push-only should write the skill to the remote archive"
    );

    Ok(())
}

#[test]
fn test_sync_pull_only() -> Result<()> {
    let mut local = setup_fixture("sync_pull_only_local")?;
    let mut remote = setup_fixture("sync_pull_only_remote")?;
    let skill_id = "pull-only-skill";

    seed_skill(
        &mut remote,
        skill_id,
        &build_skill(
            "Pull Only Skill",
            "Pull-only content provided by the remote machine",
            &["sync", "pull"],
            "Pull-only content provided by the remote machine.",
        ),
        "sync:remote_indexed",
    )?;
    add_filesystem_remote(
        &mut local,
        "pull-remote",
        &remote.ms_root,
        "sync:remote_added",
    )?;

    let report = run_sync_report(
        &mut local,
        &["--robot", "sync", "pull-remote", "--pull-only"],
        "Sync with pull-only mode",
        "sync:pull_only_complete",
    )?;
    assert!(array_contains(&report["pulled"], skill_id));
    assert_eq!(report["pushed"].as_array().map_or(0, Vec::len), 0);

    let description = load_skill_description(&mut local, skill_id)?;
    assert_eq!(
        description,
        "Pull-only content provided by the remote machine"
    );

    Ok(())
}
