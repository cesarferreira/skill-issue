use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::{fs, path::Path};

fn write_config(path: &Path, root: &Path, targets: &[(&str, &Path)]) {
    let mut contents = format!("root = {:?}\n", root);
    for (id, target) in targets {
        contents.push_str(&format!(
            "\n[targets.{id}]\npath = {:?}\nenabled = true\n",
            target
        ));
    }
    fs::write(path, contents).unwrap();
}

fn skill(path: &Path, body: &str) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("SKILL.md"), body).unwrap();
}

#[test]
fn help_lists_only_the_simple_public_workflows() {
    let output = cargo_bin_cmd!("si").arg("--help").output().unwrap();
    let help = String::from_utf8(output.stdout).unwrap();
    for command in [
        "setup", "sync", "status", "tui", "diff", "targets", "config",
    ] {
        assert!(
            help.contains(&format!("  {command}")),
            "missing {command}: {help}"
        );
    }
    for removed in [
        "init",
        "scan",
        "adopt",
        "doctor",
        "link",
        "unlink",
        "enable",
        "disable",
        "delete",
        "restore",
        "apply",
        "completions",
    ] {
        assert!(
            !help.contains(&format!("  {removed}")),
            "still exposes {removed}: {help}"
        );
    }
}

#[test]
fn removed_commands_are_rejected_without_compatibility_aliases() {
    for command in [
        "init", "scan", "adopt", "doctor", "link", "unlink", "enable", "disable", "delete",
        "restore", "apply",
    ] {
        cargo_bin_cmd!("si").arg(command).assert().failure();
    }
}

#[test]
fn missing_configuration_points_to_setup() {
    let temp = tempfile::tempdir().unwrap();
    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", temp.path().join("missing.toml"))
        .arg("status")
        .assert()
        .code(4)
        .stderr(predicate::str::contains("si setup"));
}

#[test]
fn status_names_the_canonical_root_when_it_is_not_a_git_repository() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("skills");
    fs::create_dir_all(&root).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", config)
        .args(["status", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Canonical root is not a Git repository: {}",
            root.display()
        )));
}

#[cfg(unix)]
#[test]
fn sync_reconciles_links_and_stale_managed_links() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "same");
    skill(&target.join("foo"), "same");
    std::os::unix::fs::symlink(root.join("removed"), target.join("removed")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--yes", "--no-color"])
        .assert()
        .success();
    assert!(target.join("foo").is_symlink());
    assert!(fs::symlink_metadata(target.join("removed")).is_err());
}

#[cfg(unix)]
#[test]
fn sync_force_replaces_matching_foreign_symlink_without_removing_its_target() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    let foreign = temp.path().join("foreign/foo");
    skill(&root.join("foo"), "same");
    skill(&foreign, "same");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&foreign, target.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--force", "--yes", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("REPLACE FOREIGN LINK"));

    assert_eq!(fs::read_link(target.join("foo")).unwrap(), root.join("foo"));
    assert_eq!(
        fs::read_to_string(foreign.join("SKILL.md")).unwrap(),
        "same"
    );
}

#[cfg(unix)]
#[test]
fn sync_without_force_preserves_foreign_symlink_and_suggests_force() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    let foreign = temp.path().join("foreign/foo");
    skill(&root.join("foo"), "same");
    skill(&foreign, "same");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&foreign, target.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--yes", "--no-color"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("Run: si sync --force"));

    assert_eq!(fs::read_link(target.join("foo")).unwrap(), foreign);
}

#[cfg(unix)]
#[test]
fn sync_force_repairs_broken_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "canonical");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(temp.path().join("missing"), target.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--force", "--yes", "--no-color"])
        .assert()
        .success();

    assert_eq!(fs::read_link(target.join("foo")).unwrap(), root.join("foo"));
}

#[cfg(unix)]
#[test]
fn sync_force_refuses_divergent_foreign_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    let foreign = temp.path().join("foreign/foo");
    skill(&root.join("foo"), "canonical");
    skill(&foreign, "different");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&foreign, target.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--force", "--yes", "--no-color"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("si diff foo"));

    assert_eq!(fs::read_link(target.join("foo")).unwrap(), foreign);
}

#[cfg(unix)]
#[test]
fn sync_force_dry_run_does_not_replace_foreign_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    let foreign = temp.path().join("foreign/foo");
    skill(&root.join("foo"), "same");
    skill(&foreign, "same");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&foreign, target.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--force", "--dry-run", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("REPLACE FOREIGN LINK"))
        .stdout(predicate::str::contains("Dry run; no files changed."));

    assert_eq!(fs::read_link(target.join("foo")).unwrap(), foreign);
}

#[cfg(unix)]
#[test]
fn sync_preserves_divergent_physical_content() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "canonical");
    skill(&target.join("foo"), "local");
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--yes", "--no-color"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "divergent skills require an interactive terminal",
        ));
    assert_eq!(
        fs::read_to_string(target.join("foo/SKILL.md")).unwrap(),
        "local"
    );
}

#[cfg(unix)]
#[test]
fn sync_collects_skills_after_setup_only_configures() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let claude = home.join(".claude/skills");
    let codex = home.join(".codex/skills");
    let root = home.join("code/skills");
    skill(&claude.join("claude-only"), "one");
    skill(&claude.join("shared"), "same");
    skill(&codex.join("shared"), "same");
    let config = temp.path().join("config.toml");
    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["setup", root.to_str().unwrap(), "--yes", "--no-color"])
        .assert()
        .success();
    assert!(claude.join("claude-only").is_dir());
    assert!(!root.join("claude-only").exists());

    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--yes", "--no-color"])
        .assert()
        .success();
    for name in ["claude-only", "shared"] {
        assert!(root.join(name).is_dir());
        assert!(claude.join(name).is_symlink());
        assert!(codex.join(name).is_symlink());
    }
}

#[cfg(unix)]
#[test]
fn setup_accepts_custom_targets_and_ignores_before_collection() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let root = home.join("skills");
    let extra = temp.path().join("legacy-skills");
    skill(&extra.join("collect-me"), "canonical");
    skill(&extra.join("retired"), "do not collect");
    let config = temp.path().join("config.toml");

    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILL_ISSUE_CONFIG", &config)
        .args([
            "setup",
            root.to_str().unwrap(),
            "--target",
            &format!("legacy={}", extra.display()),
            "--ignore",
            "retired",
            "--yes",
            "--no-color",
        ])
        .assert()
        .success();

    assert!(extra.join("collect-me").is_dir());
    assert!(!root.join("collect-me").exists());

    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--yes", "--no-color"])
        .assert()
        .success();

    assert!(root.join("collect-me").is_dir());
    assert!(extra.join("collect-me").is_symlink());
    assert!(extra.join("retired").is_dir());
    assert!(!root.join("retired").exists());
    let saved = fs::read_to_string(config).unwrap();
    assert!(saved.contains("[targets.legacy]"));
    assert!(saved.contains("retired"));
}

#[cfg(unix)]
#[test]
fn setup_discovers_cursor_skills() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let root = home.join("skills");
    let cursor = home.join(".cursor/skills");
    skill(&cursor.join("cursor-only"), "cursor skill");
    let config = temp.path().join("config.toml");

    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["setup", root.to_str().unwrap(), "--yes", "--no-color"])
        .assert()
        .success();
    assert!(cursor.join("cursor-only").is_dir());
    assert!(!root.join("cursor-only").exists());

    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--yes", "--no-color"])
        .assert()
        .success();

    assert!(root.join("cursor-only").is_dir());
    assert!(cursor.join("cursor-only").is_symlink());
    assert!(
        fs::read_to_string(config)
            .unwrap()
            .contains("[targets.cursor]")
    );
}

#[cfg(unix)]
#[test]
fn setup_discovers_pi_skills() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let root = home.join("skills");
    let pi = home.join(".pi/agent/skills");
    skill(&pi.join("pi-only"), "pi skill");
    let config = temp.path().join("config.toml");

    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["setup", root.to_str().unwrap(), "--yes", "--no-color"])
        .assert()
        .success();
    assert!(pi.join("pi-only").is_dir());
    assert!(!root.join("pi-only").exists());

    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--yes", "--no-color"])
        .assert()
        .success();

    assert!(root.join("pi-only").is_dir());
    assert!(pi.join("pi-only").is_symlink());
    assert!(fs::read_to_string(config).unwrap().contains("[targets.pi]"));
}

#[cfg(unix)]
#[test]
fn setup_adds_pi_to_an_existing_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let root = home.join("skills");
    let pi = home.join(".pi/agent/skills");
    skill(&pi.join("pi-only"), "pi skill");
    fs::create_dir_all(&root).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);

    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["setup", "--yes", "--no-color"])
        .assert()
        .success();

    assert!(fs::read_to_string(config).unwrap().contains("[targets.pi]"));
}

#[cfg(unix)]
#[test]
fn first_setup_dry_run_only_previews_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let root = home.join("skills");
    skill(&home.join(".claude/skills/foo"), "collect me");
    let config = temp.path().join("config.toml");

    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["setup", root.to_str().unwrap(), "--dry-run", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Run si sync"));

    assert!(!root.exists());
    assert!(!config.exists());
}

#[cfg(unix)]
#[test]
fn setup_links_an_existing_canonical_directory() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let root = home.join("code/skills");
    let claude = home.join(".claude/skills");
    skill(&root.join("foo"), "canonical");
    fs::create_dir_all(&claude).unwrap();
    let config = temp.path().join("config.toml");
    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["setup", root.to_str().unwrap(), "--yes", "--no-color"])
        .assert()
        .success();
    assert!(!claude.join("foo").is_symlink());

    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--yes", "--no-color"])
        .assert()
        .success();
    assert_eq!(
        fs::canonicalize(claude.join("foo")).unwrap(),
        fs::canonicalize(root.join("foo")).unwrap()
    );
}

#[test]
fn bare_si_is_read_only_status() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "canonical");
    skill(&target.join("foo"), "different");
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .arg("--no-color")
        .assert()
        .code(3)
        .stdout(predicate::str::contains("conflict"));
    assert_eq!(
        fs::read_to_string(target.join("foo/SKILL.md")).unwrap(),
        "different"
    );
}

#[test]
fn status_and_targets_emit_machine_readable_json() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "canonical");
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    let status = cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["--json", "status"])
        .output()
        .unwrap();
    assert_eq!(status.status.code(), Some(2));
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["skills"][0]["name"], "foo");

    let targets = cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["--json", "targets"])
        .output()
        .unwrap();
    assert!(targets.status.success());
    let targets: serde_json::Value = serde_json::from_slice(&targets.stdout).unwrap();
    assert_eq!(targets["agent"]["path"], target.to_string_lossy().as_ref());
    assert_eq!(targets["agent"]["enabled"], true);
}

#[test]
fn diff_reports_text_changes_and_json_changes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "canonical");
    skill(&target.join("foo"), "local");
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["diff", "foo", "--content", "--no-color"])
        .assert()
        .code(3)
        .stdout(predicate::str::contains("SKILL.md"))
        .stdout(predicate::str::contains("-canonical"))
        .stdout(predicate::str::contains("+local"));

    let output = cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["--json", "diff", "foo"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let diff: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(diff["skill"], "foo");
    assert_eq!(diff["changes"][0]["status"], "M");
    assert_eq!(diff["changes"][0]["path"], "SKILL.md");
}

#[test]
fn targets_add_remove_and_validation_update_only_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);
    let added = temp.path().join("new-target");

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args([
            "targets",
            "add",
            "extra",
            added.to_str().unwrap(),
            "--no-color",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Target added"));
    assert!(
        fs::read_to_string(&config)
            .unwrap()
            .contains("[targets.extra]")
    );
    assert!(!added.exists());

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["targets", "remove", "extra", "--dry-run", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run; no files changed."));
    assert!(
        fs::read_to_string(&config)
            .unwrap()
            .contains("[targets.extra]")
    );

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["targets", "remove", "extra", "--no-color"])
        .assert()
        .success();
    assert!(
        !fs::read_to_string(&config)
            .unwrap()
            .contains("[targets.extra]")
    );

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["targets", "add", "Invalid", added.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("target id must start"));
}

#[test]
fn config_commands_persist_changes_and_reject_unsafe_root_moves() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["config", "set-relative-links", "true", "--no-color"])
        .assert()
        .success();
    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["config", "add-ignore", "generated-*", "--no-color"])
        .assert()
        .success();
    let saved = fs::read_to_string(&config).unwrap();
    assert!(saved.contains("relative_links = true"));
    assert!(saved.contains("generated-*"));

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["config", "remove-ignore", "generated-*", "--no-color"])
        .assert()
        .success();
    assert!(!fs::read_to_string(&config).unwrap().contains("generated-*"));

    skill(&root.join("foo"), "canonical");
    let replacement = temp.path().join("replacement-root");
    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args([
            "config",
            "set-root",
            replacement.to_str().unwrap(),
            "--no-color",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("refusing to change the root"));
}

#[cfg(unix)]
#[test]
fn sync_creates_relative_links_when_configured() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("nested/target");
    skill(&root.join("foo"), "canonical");
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);
    fs::write(
        &config,
        format!(
            "relative_links = true\n{}",
            fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["sync", "--yes", "--no-color"])
        .assert()
        .success();
    let raw_target = fs::read_link(target.join("foo")).unwrap();
    assert!(raw_target.is_relative());
    assert_eq!(
        fs::canonicalize(target.join("foo")).unwrap(),
        fs::canonicalize(root.join("foo")).unwrap()
    );
}

#[test]
fn invalid_configuration_returns_the_configuration_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    fs::write(&config, "root = [not-a-path]").unwrap();

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", config)
        .arg("status")
        .assert()
        .code(4)
        .stderr(predicate::str::contains("Invalid configuration"));
}
