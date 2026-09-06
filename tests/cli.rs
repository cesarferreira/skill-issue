use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

fn write_config(path: &Path, root: &Path, targets: &[(&str, &Path)]) {
    let mut config = format!("root = {:?}\n", root);
    for (id, target) in targets {
        config.push_str(&format!(
            "\n[targets.{id}]\npath = {:?}\nenabled = true\n",
            target
        ));
    }
    fs::write(path, config).unwrap();
}

fn skill(path: &Path, body: &str) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("SKILL.md"), body).unwrap();
}

#[test]
fn help_lists_the_v01_commands() {
    cargo_bin_cmd!("si")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("adopt"))
        .stdout(predicate::str::contains("doctor"))
        .stdout(predicate::str::contains("targets"));
}

#[test]
fn missing_configuration_has_documented_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", temp.path().join("missing.toml"))
        .arg("status")
        .assert()
        .code(4)
        .stderr(predicate::str::contains("si init"));
}

#[test]
fn init_dry_run_does_not_write_files() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    let root = temp.path().join("skills");
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["init", root.to_str().unwrap(), "--dry-run", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run; no files changed."));
    assert!(!config.exists());
    assert!(!root.exists());
}

#[test]
fn scan_json_reports_duplicates_and_documented_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let one = temp.path().join("one");
    let two = temp.path().join("two");
    fs::create_dir_all(&root).unwrap();
    skill(&one.join("foo"), "same");
    skill(&two.join("foo"), "same");
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("one", &one), ("two", &two)]);
    let output = cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["scan", "--json", "--no-color"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["skills"][0]["status"], "identical_duplicate");
}

#[test]
fn project_scan_discovers_local_agent_skills() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let project = temp.path().join("project");
    fs::create_dir_all(&root).unwrap();
    skill(&project.join(".agents/skills/local"), "project");
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["scan", "--project", project.to_str().unwrap(), "--json"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("project-agents"))
        .stdout(predicate::str::contains("local"));
}

#[test]
fn diff_json_reports_changed_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let one = temp.path().join("one");
    let two = temp.path().join("two");
    fs::create_dir_all(&root).unwrap();
    skill(&one.join("foo"), "one");
    skill(&two.join("foo"), "two");
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("one", &one), ("two", &two)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["diff", "foo", "--content", "--json"])
        .assert()
        .code(3)
        .stdout(predicate::str::contains("SKILL.md"))
        .stdout(predicate::str::contains("@@"));
}

#[test]
fn restore_dry_run_plans_every_canonical_skill() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "body");
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["restore", "--dry-run", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("target/foo"))
        .stdout(predicate::str::contains("Dry run"));
    assert!(!target.join("foo").exists());
}

#[test]
fn completions_generate_shell_script_without_configuration() {
    cargo_bin_cmd!("si")
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_si"));
}

#[cfg(unix)]
#[test]
fn adopt_and_doctor_complete_the_real_cli_journey() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let claude = home.join(".claude/skills");
    let codex = home.join(".codex/skills");
    let root = home.join("code/skills");
    skill(&claude.join("rust skill-λ"), "same");
    skill(&codex.join("rust skill-λ"), "same");
    let config = temp.path().join("config.toml");
    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILLISSUE_CONFIG", &config)
        .args(["init", root.to_str().unwrap(), "--no-color"])
        .assert()
        .success();
    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILLISSUE_CONFIG", &config)
        .args(["adopt", "rust skill-λ", "--dry-run", "--no-color"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("STAGE"))
        .stdout(predicate::str::contains("VERIFY"))
        .stdout(predicate::str::contains("REMOVE"));
    cargo_bin_cmd!("si")
        .env("HOME", &home)
        .env("SKILLISSUE_CONFIG", &config)
        .args(["adopt", "rust skill-λ", "--yes", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No skill issues"));
    assert!(root.join("rust skill-λ").is_dir());
    assert!(
        fs::symlink_metadata(claude.join("rust skill-λ"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::symlink_metadata(codex.join("rust skill-λ"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["doctor", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No skill issues"));
}

#[cfg(unix)]
#[test]
fn link_and_unlink_protect_the_canonical_skill() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "body");
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["link", "foo", "agent", "--yes", "--no-color"])
        .assert()
        .success();
    assert!(
        fs::symlink_metadata(target.join("foo"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["unlink", "foo", "agent", "--yes", "--no-color"])
        .assert()
        .success();
    assert!(!target.join("foo").exists());
    assert!(root.join("foo").is_dir());
}

#[test]
fn status_git_json_reports_repository_health() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["status", "--git", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"repository\": true"));
}

#[test]
fn config_updates_v02_options() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["config", "set-relative-links", "true"])
        .assert()
        .success();
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["config", "add-ignore", "*.log"])
        .assert()
        .success();
    let saved = fs::read_to_string(config).unwrap();
    assert!(saved.contains("relative_links = true"));
    assert!(saved.contains("*.log"));
}

#[cfg(unix)]
#[test]
fn doctor_fix_repairs_a_stale_broken_link() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "body");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(temp.path().join("old/foo"), target.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["doctor", "--fix", "--yes", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No skill issues"));
    assert_eq!(fs::read_link(target.join("foo")).unwrap(), root.join("foo"));
}

#[test]
fn doctor_json_reports_interrupted_migration_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(root.join(".skillissue-backup-foo-1-1")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["doctor", "--json"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("recovery_artifact"));
}

#[test]
fn assume_yes_still_refuses_noninteractive_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let one = temp.path().join("one");
    let two = temp.path().join("two");
    fs::create_dir_all(&root).unwrap();
    skill(&one.join("foo"), "one");
    skill(&two.join("foo"), "two");
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("one", &one), ("two", &two)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["adopt", "foo", "--yes", "--no-color"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("require an interactive terminal"));
    assert!(one.join("foo").is_dir());
    assert!(two.join("foo").is_dir());
    assert!(!root.join("foo").exists());
}

#[cfg(unix)]
#[test]
fn configured_relative_links_are_used_by_link_command() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("nested/target");
    skill(&root.join("foo"), "body");
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["config", "set-relative-links", "true"])
        .assert()
        .success();
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["link", "foo", "agent", "--yes"])
        .assert()
        .success();
    assert!(fs::read_link(target.join("foo")).unwrap().is_relative());
}
