#![cfg(unix)]

use assert_cmd::cargo::cargo_bin;
use expectrl::{Eof, Expect, Session};
use std::fs;
use std::process::Command;
use std::time::Duration;

fn skill(path: &std::path::Path, body: &str) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("SKILL.md"), body).unwrap();
}

#[test]
fn divergent_interactive_flow_can_diff_choose_and_preserve() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    let cache = temp.path().join("cache");
    fs::create_dir_all(&root).unwrap();
    skill(&claude.join("android"), "claude version\n");
    skill(&codex.join("android"), "codex version\n");
    let config = temp.path().join("config.toml");
    fs::write(
        &config,
        format!(
            "root = {:?}\n\n[targets.claude]\npath = {:?}\nenabled = true\n\n[targets.codex]\npath = {:?}\nenabled = true\n",
            root, claude, codex
        ),
    )
    .unwrap();

    let mut command = Command::new(cargo_bin("si"));
    command
        .env("SKILLISSUE_CONFIG", &config)
        .env("SKILLISSUE_CACHE", &cache)
        .args(["adopt", "android", "--no-color"]);
    let mut session = Session::spawn(command).unwrap();
    session.set_expect_timeout(Some(Duration::from_secs(10)));
    session.expect("copies differ").unwrap();
    session.expect("android has divergent copies").unwrap();

    // The menu defaults to Skip. Move to View diff and select it.
    session.send("\x1b[A\r").unwrap();
    session.expect("SKILL.md").unwrap();
    session.expect("android has divergent copies").unwrap();

    // Return from Skip to the first concrete version and select it.
    session.send("\x1b[A\x1b[A\x1b[A\x1b[A\r").unwrap();
    session.expect("Use this resolution?").unwrap();
    session.send_line("y").unwrap();
    session.expect("divergent versions preserved").unwrap();
    session.expect("No skill issues").unwrap();
    session.expect(Eof).unwrap();

    assert!(root.join("android").is_dir());
    assert!(claude.join("android").is_symlink());
    assert!(codex.join("android").is_symlink());
    let migration_root = cache.join("skillissue/migrations");
    let timestamp = fs::read_dir(migration_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let archived_targets = fs::read_dir(timestamp.join("android"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .count();
    assert_eq!(archived_targets, 1);
}

#[test]
fn identical_skills_are_confirmed_individually_and_skips_continue() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    fs::create_dir_all(&root).unwrap();
    for name in ["a-skip", "b-adopt"] {
        skill(&claude.join(name), "same\n");
        skill(&codex.join(name), "same\n");
    }
    let config = temp.path().join("config.toml");
    fs::write(
        &config,
        format!(
            "root = {:?}\n\n[targets.claude]\npath = {:?}\nenabled = true\n\n[targets.codex]\npath = {:?}\nenabled = true\n",
            root, claude, codex
        ),
    )
    .unwrap();
    let mut command = Command::new(cargo_bin("si"));
    command
        .env("SKILLISSUE_CONFIG", &config)
        .args(["adopt", "--no-color"]);
    let mut session = Session::spawn(command).unwrap();
    session.set_expect_timeout(Some(Duration::from_secs(10)));
    session.expect("a-skip").unwrap();
    session.expect("Adopt?").unwrap();
    session.send_line("n").unwrap();
    session.expect("b-adopt").unwrap();
    session.expect("Adopt?").unwrap();
    session.send_line("y").unwrap();
    session.expect("1 adopted").unwrap();
    session.expect("1 skipped").unwrap();
    session.expect(Eof).unwrap();
    assert!(claude.join("a-skip").is_dir());
    assert!(codex.join("a-skip").is_dir());
    assert!(root.join("b-adopt").is_dir());
    assert!(claude.join("b-adopt").is_symlink());
    assert!(codex.join("b-adopt").is_symlink());
}

#[test]
fn keep_both_creates_separate_canonical_skills() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    fs::create_dir_all(&root).unwrap();
    skill(&claude.join("android"), "claude\n");
    skill(&codex.join("android"), "codex\n");
    let config = temp.path().join("config.toml");
    fs::write(
        &config,
        format!(
            "root = {:?}\n\n[targets.claude]\npath = {:?}\nenabled = true\n\n[targets.codex]\npath = {:?}\nenabled = true\n",
            root, claude, codex
        ),
    )
    .unwrap();
    let mut command = Command::new(cargo_bin("si"));
    command
        .env("SKILLISSUE_CONFIG", &config)
        .args(["adopt", "android", "--no-color"]);
    let mut session = Session::spawn(command).unwrap();
    session.set_expect_timeout(Some(Duration::from_secs(10)));
    session.expect("android has divergent copies").unwrap();
    // Skip -> View diff -> Keep all versions.
    session.send("\x1b[A\x1b[A\r").unwrap();
    session.expect("Canonical name for").unwrap();
    session.send_line("").unwrap();
    session.expect("Canonical name for").unwrap();
    session.send_line("").unwrap();
    session.expect("Use this resolution?").unwrap();
    session.send_line("y").unwrap();
    session.expect("android adopted").unwrap();
    session.expect(Eof).unwrap();
    let canonical_names: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(canonical_names.len(), 2);
    assert!(claude.join("android").is_symlink());
    assert!(codex.join("android").is_symlink());
}

#[test]
fn bootstrap_link_all_asks_once_and_uses_every_detected_agent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    skill(&root.join("android"), "android\n");
    skill(&root.join("rust-cli"), "rust\n");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = temp.path().join("config.toml");
    fs::write(
        &config,
        format!(
            "root = {:?}\n\n[targets.claude]\npath = {:?}\nenabled = true\n\n[targets.codex]\npath = {:?}\nenabled = true\n",
            root, claude, codex
        ),
    )
    .unwrap();
    let mut command = Command::new(cargo_bin("si"));
    command
        .env("SKILLISSUE_CONFIG", &config)
        .args(["link", "--all", "--no-color"]);
    let mut session = Session::spawn(command).unwrap();
    session.set_expect_timeout(Some(Duration::from_secs(10)));
    session.expect("Detected:").unwrap();
    session
        .expect("Link 2 skills into all detected agents?")
        .unwrap();
    session.send_line("").unwrap();
    session.expect("4 links created").unwrap();
    session.expect("No skill issues").unwrap();
    session.expect(Eof).unwrap();
}
