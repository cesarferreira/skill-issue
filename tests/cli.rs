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
