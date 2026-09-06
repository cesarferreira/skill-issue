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

fn git(path: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_sync_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let writer = temp.path().join("writer");
    let machine = temp.path().join("machine");
    fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "--bare", "-q"]);
    git(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git(
        temp.path(),
        &["clone", "-q", remote.to_str().unwrap(), "writer"],
    );
    skill(&writer.join("foo"), "first");
    git(&writer, &["add", "."]);
    git(
        &writer,
        &[
            "-c",
            "user.name=Skill Issue Tests",
            "-c",
            "user.email=tests@example.com",
            "commit",
            "-qm",
            "initial",
        ],
    );
    git(&writer, &["push", "-qu", "origin", "HEAD:main"]);
    git(
        temp.path(),
        &["clone", "-q", remote.to_str().unwrap(), "machine"],
    );
    (temp, writer, machine)
}

#[test]
fn help_lists_the_v01_commands() {
    cargo_bin_cmd!("si")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("tui"))
        .stdout(predicate::str::contains("adopt"))
        .stdout(predicate::str::contains("doctor"))
        .stdout(predicate::str::contains("disable"))
        .stdout(predicate::str::contains("enable"))
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("targets"));
}

#[test]
fn tui_refuses_noninteractive_output_without_corrupting_it() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .arg("tui")
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires an interactive terminal"))
        .stdout(predicate::str::is_empty());
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
fn current_configuration_environment_variable_is_supported() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);
    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", config)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("skill-issue"));
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

#[cfg(unix)]
#[test]
fn apply_links_every_canonical_skill_into_every_target() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    skill(&root.join("foo"), "body");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("claude", &claude), ("codex", &codex)]);

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["apply", "--yes", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 links created"));

    let canonical = fs::canonicalize(root.join("foo")).unwrap();
    assert_eq!(fs::canonicalize(claude.join("foo")).unwrap(), canonical);
    assert_eq!(fs::canonicalize(codex.join("foo")).unwrap(), canonical);
}

#[cfg(unix)]
#[test]
fn apply_removes_stale_managed_links_after_a_skill_is_deleted() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(root.join("removed"), target.join("removed")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["apply", "--yes", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 stale link removed"));

    assert!(fs::symlink_metadata(target.join("removed")).is_err());
}

#[cfg(unix)]
#[test]
fn apply_replaces_an_identical_physical_copy_with_a_link() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "same");
    skill(&target.join("foo"), "same");
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["apply", "--yes", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 link created"));

    assert!(target.join("foo").is_symlink());
    assert_eq!(
        fs::canonicalize(target.join("foo")).unwrap(),
        fs::canonicalize(root.join("foo")).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn apply_preserves_a_divergent_physical_copy() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "canonical");
    skill(&target.join("foo"), "local");
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILL_ISSUE_CONFIG", &config)
        .args(["apply", "--yes", "--no-color"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("si setup"));

    assert_eq!(
        fs::read_to_string(target.join("foo/SKILL.md")).unwrap(),
        "local"
    );
    assert!(!target.join("foo").is_symlink());
}

#[cfg(unix)]
#[test]
fn setup_collects_existing_skills_and_links_every_detected_agent() {
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
        .success()
        .stdout(predicate::str::contains("No skill issues"));

    for name in ["claude-only", "shared"] {
        assert!(root.join(name).is_dir());
        assert!(claude.join(name).is_symlink());
        assert!(codex.join(name).is_symlink());
    }
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

    assert_eq!(
        fs::canonicalize(claude.join("foo")).unwrap(),
        fs::canonicalize(root.join("foo")).unwrap()
    );
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

#[cfg(unix)]
#[test]
fn disable_and_enable_toggle_every_managed_agent_link() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    skill(&root.join("foo"), "body");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    std::os::unix::fs::symlink(root.join("foo"), claude.join("foo")).unwrap();
    std::os::unix::fs::symlink(root.join("foo"), codex.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("claude", &claude), ("codex", &codex)]);

    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["disable", "foo", "--yes", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("disabled for 2 agents"));
    assert!(!claude.join("foo").exists());
    assert!(!codex.join("foo").exists());
    assert!(root.join("foo").is_dir());

    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["enable", "foo", "--yes", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("enabled for 2 agents"));
    assert!(claude.join("foo").is_symlink());
    assert!(codex.join("foo").is_symlink());
}

#[cfg(unix)]
#[test]
fn disable_dry_run_previews_without_removing_links() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "body");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(root.join("foo"), target.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["disable", "foo", "--dry-run", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DISABLE"))
        .stdout(predicate::str::contains("Dry run; no files changed."));
    assert!(target.join("foo").is_symlink());
}

#[cfg(unix)]
#[test]
fn toggles_refuse_foreign_links_and_physical_copies() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    let foreign = temp.path().join("foreign");
    skill(&root.join("foo"), "canonical");
    skill(&foreign, "foreign");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&foreign, target.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["disable", "foo", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "refusing to remove foreign symlink",
        ));
    assert!(target.join("foo").is_symlink());

    fs::remove_file(target.join("foo")).unwrap();
    skill(&target.join("foo"), "physical");
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["enable", "foo", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "refusing to replace physical directory",
        ));
    assert!(target.join("foo").is_dir());
}

#[test]
fn toggles_reject_missing_canonical_skills() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["enable", "missing", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("canonical skill does not exist"));
}

#[cfg(unix)]
#[test]
fn delete_removes_links_and_the_canonical_skill() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    skill(&root.join("foo"), "body");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    std::os::unix::fs::symlink(root.join("foo"), claude.join("foo")).unwrap();
    std::os::unix::fs::symlink(root.join("foo"), codex.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("claude", &claude), ("codex", &codex)]);

    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["delete", "foo", "--yes", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted"));
    assert!(!claude.join("foo").exists());
    assert!(!codex.join("foo").exists());
    assert!(!root.join("foo").exists());
}

#[cfg(unix)]
#[test]
fn delete_dry_run_previews_without_removing_anything() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    skill(&root.join("foo"), "body");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(root.join("foo"), target.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["delete", "foo", "--dry-run", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DELETE"))
        .stdout(predicate::str::contains("Dry run; no files changed."));
    assert!(target.join("foo").is_symlink());
    assert!(root.join("foo").is_dir());
}

#[test]
fn delete_rejects_missing_canonical_skills() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("target");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["delete", "missing", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("canonical skill does not exist"));
}

#[test]
fn enable_requires_at_least_one_enabled_target() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    skill(&root.join("foo"), "body");
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["enable", "foo", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no enabled targets were detected"));
}

#[test]
fn tui_rejects_json_mode_before_touching_the_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["tui", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--json cannot be combined with tui",
        ));
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

#[cfg(unix)]
#[test]
fn sync_pulls_remote_skills_and_repairs_all_links() {
    let (temp, writer, machine) = git_sync_fixture();
    skill(&writer.join("bar"), "second");
    git(&writer, &["add", "."]);
    git(
        &writer,
        &[
            "-c",
            "user.name=Skill Issue Tests",
            "-c",
            "user.email=tests@example.com",
            "commit",
            "-qm",
            "add bar",
        ],
    );
    git(&writer, &["push", "-q"]);

    let target = temp.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &machine, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["sync", "--check", "--no-color"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains(
            "remote commit(s) have not been pulled",
        ));

    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["sync", "--yes", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Canonical repository updated"))
        .stdout(predicate::str::contains("This computer is in sync"));
    assert!(machine.join("bar").is_dir());
    assert!(target.join("foo").is_symlink());
    assert!(target.join("bar").is_symlink());

    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["sync", "--check", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("This computer is in sync"));
}

#[test]
fn sync_check_json_explains_local_link_drift() {
    let (temp, _writer, machine) = git_sync_fixture();
    let target = temp.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &machine, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["sync", "--check", "--json"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("\"synced\": false"))
        .stdout(predicate::str::contains("\"links_healthy\": false"))
        .stdout(predicate::str::contains(
            "one or more agent links need repair",
        ));
}

#[test]
fn sync_refuses_non_git_canonical_roots() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["sync", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a Git repository"));
}

#[test]
fn sync_refuses_a_canonical_directory_nested_inside_another_repository() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q"]);
    let root = temp.path().join("skills");
    fs::create_dir_all(&root).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["sync", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a Git repository"));
}

#[test]
fn sync_refuses_dirty_repositories_before_pulling_or_linking() {
    let (temp, _writer, machine) = git_sync_fixture();
    fs::write(machine.join("foo/SKILL.md"), "local edit").unwrap();
    let target = temp.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &machine, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["sync", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("uncommitted changes"));
    assert!(!target.join("foo").exists());
}

#[test]
fn sync_check_reports_a_missing_upstream() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q"]);
    skill(&root.join("foo"), "body");
    git(&root, &["add", "."]);
    git(
        &root,
        &[
            "-c",
            "user.name=Skill Issue Tests",
            "-c",
            "user.email=tests@example.com",
            "commit",
            "-qm",
            "initial",
        ],
    );
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["sync", "--check", "--no-color"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("no upstream"));
}

#[test]
fn sync_reports_local_commits_that_still_need_pushing() {
    let (temp, _writer, machine) = git_sync_fixture();
    fs::write(machine.join("foo/SKILL.md"), "local commit").unwrap();
    git(&machine, &["add", "."]);
    git(
        &machine,
        &[
            "-c",
            "user.name=Skill Issue Tests",
            "-c",
            "user.email=tests@example.com",
            "commit",
            "-qm",
            "local",
        ],
    );
    let target = temp.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &machine, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["sync", "--yes", "--no-color"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("have not been pushed"));
    assert!(target.join("foo").is_symlink());
}

#[test]
fn sync_refuses_diverged_histories() {
    let (temp, writer, machine) = git_sync_fixture();
    skill(&writer.join("remote-only"), "remote");
    git(&writer, &["add", "."]);
    git(
        &writer,
        &[
            "-c",
            "user.name=Skill Issue Tests",
            "-c",
            "user.email=tests@example.com",
            "commit",
            "-qm",
            "remote",
        ],
    );
    git(&writer, &["push", "-q"]);
    skill(&machine.join("local-only"), "local");
    git(&machine, &["add", "."]);
    git(
        &machine,
        &[
            "-c",
            "user.name=Skill Issue Tests",
            "-c",
            "user.email=tests@example.com",
            "commit",
            "-qm",
            "local",
        ],
    );
    let target = temp.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &machine, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["sync", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("has diverged"));
    assert!(!machine.join("remote-only").exists());
    assert!(!target.join("foo").exists());
}

#[test]
fn sync_check_reports_fetch_failures_without_touching_links() {
    let (temp, _writer, machine) = git_sync_fixture();
    fs::rename(
        temp.path().join("remote.git"),
        temp.path().join("offline.git"),
    )
    .unwrap();
    let target = temp.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &machine, &[("agent", &target)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["sync", "--check"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("fetch upstream failed"));
    assert!(!target.join("foo").exists());
}

#[test]
fn sync_dry_run_does_not_fetch_pull_or_link() {
    let (temp, writer, machine) = git_sync_fixture();
    skill(&writer.join("bar"), "second");
    git(&writer, &["add", "."]);
    git(
        &writer,
        &[
            "-c",
            "user.name=Skill Issue Tests",
            "-c",
            "user.email=tests@example.com",
            "commit",
            "-qm",
            "add bar",
        ],
    );
    git(&writer, &["push", "-q"]);
    let before = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&machine)
        .output()
        .unwrap()
        .stdout;
    let target = temp.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &machine, &[("agent", &target)]);

    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["sync", "--dry-run", "--no-color"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("no remote refs fetched"));
    let after = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&machine)
        .output()
        .unwrap()
        .stdout;
    assert_eq!(before, after);
    assert!(!machine.join("bar").exists());
    assert!(!target.join("foo").exists());
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

#[test]
fn scan_prints_the_compact_happy_path_summary() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    fs::create_dir_all(&root).unwrap();
    skill(&claude.join("same"), "same");
    skill(&codex.join("same"), "same");
    skill(&claude.join("different"), "one");
    skill(&codex.join("different"), "two");
    skill(&claude.join("unique"), "only");
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("claude", &claude), ("codex", &codex)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["scan", "--no-color"])
        .assert()
        .code(3)
        .stdout(predicate::str::contains("Scanning known skill directories"))
        .stdout(predicate::str::contains("Found 5 installations"))
        .stdout(predicate::str::contains("Found 3 unique skills"))
        .stdout(predicate::str::contains("1 identical duplicates"))
        .stdout(predicate::str::contains("1 divergent"))
        .stdout(predicate::str::contains("1 unique"));
}

#[cfg(unix)]
#[test]
fn adopt_links_a_skill_into_every_detected_target_and_reports_totals() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    let gemini = temp.path().join("gemini");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&gemini).unwrap();
    skill(&claude.join("foo"), "same");
    skill(&codex.join("foo"), "same");
    let config = temp.path().join("config.toml");
    write_config(
        &config,
        &root,
        &[("claude", &claude), ("codex", &codex), ("gemini", &gemini)],
    );
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["adopt", "foo", "--yes", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 skills"))
        .stdout(predicate::str::contains("3 symlinks created"))
        .stdout(predicate::str::contains("no data lost"))
        .stdout(predicate::str::contains("No skill issues"));
    for target in [&claude, &codex, &gemini] {
        assert!(
            fs::symlink_metadata(target.join("foo"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

#[cfg(unix)]
#[test]
fn link_all_without_targets_uses_every_detected_target() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    skill(&root.join("foo"), "one");
    skill(&root.join("bar"), "two");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("claude", &claude), ("codex", &codex)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["link", "--all", "--yes", "--no-color"])
        .assert()
        .success()
        .stdout(predicate::str::contains("4 links created"))
        .stdout(predicate::str::contains("No skill issues"));
    for target in [&claude, &codex] {
        for name in ["foo", "bar"] {
            assert!(
                fs::symlink_metadata(target.join(name))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn link_one_skill_all_targets_and_unlink_target_flag_are_supported() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    skill(&root.join("foo"), "one");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("claude", &claude), ("codex", &codex)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["link", "foo", "--all", "--yes"])
        .assert()
        .success();
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .args(["unlink", "foo", "--target", "codex", "--yes"])
        .assert()
        .success();
    assert!(claude.join("foo").exists());
    assert!(!codex.join("foo").exists());
}

#[test]
fn help_is_themed_unless_colour_is_declined() {
    cargo_bin_cmd!("si")
        .env_remove("NO_COLOR")
        .env("CLICOLOR_FORCE", "1")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{1b}[38;5;141mUsage:"));
    for args in [
        vec!["--help", "--no-color"],
        vec!["--help"], // NO_COLOR stays set below
    ] {
        let mut command = cargo_bin_cmd!("si");
        command.env("CLICOLOR_FORCE", "1").args(&args);
        if args.contains(&"--no-color") {
            command.env_remove("NO_COLOR");
        } else {
            command.env("NO_COLOR", "1");
        }
        command
            .assert()
            .success()
            .stdout(predicate::str::contains("\u{1b}[").not())
            .stdout(predicate::str::contains("Usage: si"));
    }
}

#[cfg(unix)]
#[test]
fn status_paints_the_dashboard_when_the_terminal_supports_colour() {
    let (temp, config, _) = coverage_fixture();
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .env_remove("NO_COLOR")
        .env("CLICOLOR_FORCE", "1")
        .arg("status")
        .assert()
        .code(2)
        .stdout(predicate::str::contains("\u{1b}[38;5;"))
        .stdout(predicate::str::contains("█"))
        .stdout(predicate::str::contains("Claude"));
    drop(temp);
}

#[cfg(unix)]
#[test]
fn no_color_and_the_no_color_variable_both_keep_output_free_of_escapes() {
    let (temp, config, _) = coverage_fixture();
    for (args, no_color_env) in [
        (vec!["status", "--no-color"], None),
        (vec!["status"], Some("1")),
    ] {
        let mut command = cargo_bin_cmd!("si");
        command
            .env("SKILLISSUE_CONFIG", &config)
            .env("CLICOLOR_FORCE", "1")
            .args(&args);
        match no_color_env {
            Some(value) => command.env("NO_COLOR", value),
            None => command.env_remove("NO_COLOR"),
        };
        command
            .assert()
            .code(2)
            .stdout(predicate::str::contains("\u{1b}[").not())
            .stdout(predicate::str::contains("█").not())
            .stdout(predicate::str::contains("Claude     1/1"));
    }
    drop(temp);
}

#[cfg(unix)]
#[test]
fn failures_keep_their_colour_off_stderr_when_colour_is_disabled() {
    let (temp, config, _) = coverage_fixture();
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", &config)
        .env_remove("NO_COLOR")
        .env("CLICOLOR_FORCE", "1")
        .args(["unlink", "missing", "--target", "claude", "--no-color"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("\u{1b}[").not())
        .stderr(predicate::str::contains("Error:"));
    drop(temp);
}

#[cfg(unix)]
fn coverage_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    skill(&root.join("foo"), "one");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    std::os::unix::fs::symlink(root.join("foo"), claude.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("claude", &claude), ("codex", &codex)]);
    (temp, config, root)
}

#[cfg(unix)]
#[test]
fn status_reports_per_agent_canonical_coverage() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    skill(&root.join("foo"), "one");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    std::os::unix::fs::symlink(root.join("foo"), claude.join("foo")).unwrap();
    let config = temp.path().join("config.toml");
    write_config(&config, &root, &[("claude", &claude), ("codex", &codex)]);
    cargo_bin_cmd!("si")
        .env("SKILLISSUE_CONFIG", config)
        .args(["status", "--no-color"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("Claude     1/1"))
        .stdout(predicate::str::contains("Codex      0/1"));
}
