use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

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
