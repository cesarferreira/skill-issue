# Simple CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the operation-oriented CLI with a simple `setup`, `apply`, `status`, and optional `tui` workflow while preserving the existing safe migration engine and TUI capabilities.

**Architecture:** Add focused setup and apply workflow modules that build complete plans before mutation and reuse the existing discovery, fingerprinting, staging, rollback, and symlink primitives. Cut the public Clap surface over only after those workflows have end-to-end coverage, make bare `si` a read-only status alias, and keep Git operations read-only.

**Tech Stack:** Rust 1.85+, clap 4, anyhow, dialoguer, BLAKE3, tempfile, assert_cmd, expectrl, ratatui/crossterm.

**Spec:** `docs/superpowers/specs/2026-09-06-simple-cli-design.md`

## Global Constraints

- Preserve the user's existing uncommitted changes in `README.md`, `src/cli.rs`, `src/lib.rs`, `src/tui.rs`, and `tests/cli.rs`; they add TUI deletion and visual refinements.
- Remove the public `delete` command, but retain `SkillDeletePlan`, `plan_skill_delete`, TUI deletion, and their unit/TUI coverage.
- Do not retain aliases for removed commands: `init`, `scan`, `adopt`, `doctor`, `link`, `unlink`, `enable`, `disable`, `delete`, `restore`, and `sync` must fail Clap parsing.
- `skill-issue` must never execute skill contents or silently overwrite divergent content.
- `setup` and `apply` must not execute `git init`, `git fetch`, `git pull`, `git commit`, or `git push`.
- Bare `si` and `si status` are always read-only.
- Keep config and target state machine-local; never write configuration into the canonical skills repository.
- Every filesystem mutation has an exact dry-run representation and must be verified after execution.

## File Map

- Create `src/apply.rs`: deterministic canonical-to-agent reconciliation planning, rendering, execution, and rollback.
- Create `src/setup.rs`: configuration bootstrap, target detection, conflict decisions, aggregate migration planning, execution, and verification.
- Create `src/transaction.rs`: shared mutation journal used to commit or roll back multi-skill setup and apply plans.
- Modify `src/cli.rs`: define the final public commands and their arguments.
- Modify `src/lib.rs`: expose shared crate-private primitives, dispatch the new workflows, make status the default, retain read-only Git health, and remove obsolete command implementations.
- Modify `src/tui.rs`: keep all current features while changing guidance to `setup`/`apply` and reusing shared plan primitives.
- Modify `tests/cli.rs`: replace obsolete command tests with end-to-end setup/apply/status tests.
- Modify `tests/interactive.rs`: move interactive adoption coverage to interactive setup coverage.
- Modify `README.md`: lead with first-machine, second-machine, and daily workflows; keep the TUI optional.
- Modify `prd.md`: make the product promise, command list, primary journey, restore story, roadmap, and TUI status match the approved design.

---

### Task 1: Implement deterministic `si apply` reconciliation

**Files:**
- Create: `src/apply.rs`
- Create: `src/transaction.rs`
- Modify: `src/lib.rs`
- Modify: `src/cli.rs`
- Test: `tests/cli.rs`

**Interfaces:**
- Consumes: `Config`, `ScanResult`, `SkillGroup`, `InstallationKind`, `fingerprint_with_ignores`, `managed_link_value`, `verify_link`, `resolve_link_path`, and existing migration staging helpers from `src/lib.rs`.
- Produces: `apply::run(config: &Config, dry_run: bool) -> anyhow::Result<u8>`, `apply::build_plan(config: &Config) -> anyhow::Result<ApplyPlan>`, `ApplyPlan::execute(&self, config: &Config, transaction: &mut MutationTransaction) -> anyhow::Result<()>`, and the shared `MutationTransaction` journal.

- [ ] **Step 1: Add failing CLI tests for link creation and stale-link cleanup**

Append tests using the existing `skill` and `write_config` helpers:

```rust
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
```

- [ ] **Step 2: Run the new tests and verify that Clap rejects `apply`**

Run:

```bash
cargo test --test cli apply_links_every_canonical_skill_into_every_target -- --exact
cargo test --test cli apply_removes_stale_managed_links_after_a_skill_is_deleted -- --exact
```

Expected: both fail because `apply` is not yet a recognized subcommand.

- [ ] **Step 3: Add the apply command and plan model**

Add `Apply` to `Command`, `mod apply;` to `src/lib.rs`, and dispatch it with the already-loaded config:

```rust
Command::Apply => apply::run(&config, cli.dry_run),
```

Define this plan model in `src/apply.rs`:

```rust
#[derive(Clone, Debug)]
pub(crate) struct ApplyPlan {
    pub(crate) actions: Vec<ApplyAction>,
    pub(crate) conflicts: Vec<ApplyConflict>,
}

#[derive(Clone, Debug)]
pub(crate) enum ApplyAction {
    CreateTarget { path: PathBuf },
    CreateLink { canonical: PathBuf, link: PathBuf },
    ReplaceIdentical {
        canonical: PathBuf,
        physical: PathBuf,
        backup: PathBuf,
    },
    RepairManagedLink {
        canonical: PathBuf,
        link: PathBuf,
        previous_target: PathBuf,
    },
    RemoveStaleManagedLink { link: PathBuf, previous_target: PathBuf },
}

#[derive(Clone, Debug)]
pub(crate) struct ApplyConflict {
    pub(crate) path: PathBuf,
    pub(crate) reason: String,
}
```

`build_plan` must iterate every enabled target and every real immediate child directory of the canonical root. Classify each destination with `symlink_metadata`:

- missing: `CreateLink`;
- verified link to the canonical skill: no action;
- broken symlink whose resolved target is inside the canonical root and has the same basename: `RepairManagedLink`;
- physical directory with the same fingerprint: `ReplaceIdentical` using a unique migration backup path;
- divergent physical directory or foreign symlink: `ApplyConflict`.

After canonical skills, inspect every target child. A broken symlink whose resolved target is directly inside the canonical root and whose target no longer exists becomes `RemoveStaleManagedLink`. Do not classify arbitrary broken or foreign symlinks as stale managed links.

- [ ] **Step 4: Implement rendering, confirmation, execution, rollback, and verification**

Implement:

```rust
pub(crate) fn run(config: &Config, dry_run: bool) -> Result<u8> {
    let plan = build_plan(config)?;
    plan.render(config);
    if dry_run {
        println!("{}", theme::dim("Dry run; no files changed."));
        return Ok(if plan.conflicts.is_empty() { EXIT_OK } else { EXIT_ISSUES });
    }
    if !plan.actions.is_empty() {
        require_confirmation_with_default("Apply this plan?", true)?;
        plan.execute(config)?;
    }
    let after = scan(config)?;
    render_apply_result(&after, &plan)
}
```

Implement `src/transaction.rs` with these interfaces:

```rust
pub(crate) struct MutationTransaction {
    undo: Vec<UndoAction>,
    cleanup: Vec<PathBuf>,
}

pub(crate) enum UndoAction {
    RemoveLink { path: PathBuf },
    RestoreSymlink { path: PathBuf, raw_target: PathBuf },
    RestoreDirectory { original: PathBuf, backup: PathBuf },
    RemoveDirectoryIfEmpty { path: PathBuf },
}

impl MutationTransaction {
    pub(crate) fn new() -> Self;
    pub(crate) fn record(&mut self, action: UndoAction);
    pub(crate) fn cleanup_on_commit(&mut self, path: PathBuf);
    pub(crate) fn commit(self) -> Result<()>;
    pub(crate) fn rollback(&mut self) -> Result<()>;
}
```

`ApplyPlan::execute` receives the shared journal rather than committing it. Perform target creation first, stage identical physical directories second, remove/replace links third, and verify all final links. Register backups for commit cleanup instead of deleting them immediately. `apply::run` commits after verification; on error it calls `rollback`. Rollback reverses the journal: remove newly created links, recreate removed links with their previous raw target, restore staged physical directories, and remove target directories only when this invocation created them and they remain empty.

Do not execute any action when `conflicts` is non-empty. Render every conflict and return `EXIT_ISSUES` with guidance to run `si setup` or `si diff <skill>`.

- [ ] **Step 5: Add and pass safety tests**

Add tests for dry-run, identical replacement, divergent preservation, foreign-symlink preservation, target creation, and rollback. The divergent assertion must prove the physical file is unchanged:

```rust
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
        .stdout(predicate::str::contains("si diff foo"));

    assert_eq!(fs::read_to_string(target.join("foo/SKILL.md")).unwrap(), "local");
    assert!(!target.join("foo").is_symlink());
}
```

Run:

```bash
cargo test --test cli apply -- --nocapture
```

Expected: all apply tests pass.

- [ ] **Step 6: Commit the apply workflow**

```bash
git add src/apply.rs src/transaction.rs
git add -p src/cli.rs src/lib.rs tests/cli.rs
git commit -m "feat: add local skill apply workflow"
```

---

### Task 2: Implement the aggregate `si setup` workflow

**Files:**
- Create: `src/setup.rs`
- Modify: `src/lib.rs`
- Modify: `src/cli.rs`
- Test: `tests/cli.rs`
- Test: `tests/interactive.rs`

**Interfaces:**
- Consumes: `Config::load`, `Config::save`, `Config::path`, `validate_config`, `scan`, `plans_for_group`, `add_missing_target_links`, `render_adoption_plans`, and the low-level adoption transaction helpers in `src/lib.rs`.
- Produces: `setup::run(root: Option<PathBuf>, dry_run: bool) -> anyhow::Result<u8>`, `setup::build_plan(config: Config, write_config: bool) -> anyhow::Result<SetupPlan>`, and `execute_adoption_plans(plans: &[AdoptionPlan], transaction: &mut MutationTransaction) -> anyhow::Result<()>`.

- [ ] **Step 1: Add failing first-machine and cloned-repository tests**

Add:

```rust
#[cfg(unix)]
#[test]
fn setup_collects_all_existing_skills_and_links_every_agent() {
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
fn setup_links_an_existing_cloned_repository() {
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
```

- [ ] **Step 2: Run the tests and verify they fail because setup is absent**

Run:

```bash
cargo test --test cli setup_collects_all_existing_skills_and_links_every_agent -- --exact
cargo test --test cli setup_links_an_existing_cloned_repository -- --exact
```

Expected: Clap reports `setup` as an unknown subcommand.

- [ ] **Step 3: Add setup configuration and target detection**

Add `Setup { root: Option<PathBuf> }` to `Command` and dispatch before mandatory config loading:

```rust
Some(Command::Setup { root }) => setup::run(root, cli.dry_run),
```

Define:

```rust
#[derive(Debug)]
pub(crate) struct SetupPlan {
    pub(crate) config: Config,
    pub(crate) config_changed: bool,
    pub(crate) create_root: bool,
    pub(crate) adoptions: Vec<AdoptionPlan>,
    pub(crate) reconciliation: ApplyPlan,
}
```

Extract the known target table from the old `init` function into one shared function:

```rust
pub(crate) fn detected_targets(home: &Path) -> BTreeMap<String, TargetConfig>
```

It must detect the existing Claude, Codex, Gemini, `.agents`, OpenCode, and Hermes directories. Merge detections into existing config with `entry(id).or_insert(target)` so rerunning setup never removes or overwrites custom target configuration.

When config is absent, synthesize it in memory. When `--dry-run` is set, do not create the root or save config. When config exists and a different explicit root is supplied, return: `configuration already uses <old>; run si config set-root <new> first`.

- [ ] **Step 4: Build one setup plan before mutation**

Use the effective in-memory config to scan. For each group containing physical installations, gather divergent choices with the existing dialoguer flow, then build `AdoptionPlan` values with `plans_for_group`. Call `add_missing_target_links` for each single-version plan so unique skills become available to every enabled agent.

Also call `apply::build_plan` to capture missing links for canonical-only skills and stale managed links. Add `ApplyAction::affected_path(&self) -> &Path` and filter reconciliation actions and conflicts whose paths already appear in an adoption plan's replacements. A resolved setup conflict therefore belongs to the adoption plan only; an unresolved or skipped conflict remains in `SetupPlan::reconciliation.conflicts` and prevents mutation.

Do not prompt `Adopt?` per skill. Render all `AdoptionPlan` values beneath one `SETUP PLAN` heading. After all conflict choices are known, ask only:

```text
Apply this setup plan? [Y/n]
```

For a non-interactive divergent group, return an error naming the skill and preserve every copy. `--yes` bypasses only the final confirmation.

- [ ] **Step 5: Refactor adoption execution into an aggregate transaction**

Replace the per-plan-only executor with:

```rust
pub(crate) fn execute_adoption_plans(
    plans: &[AdoptionPlan],
    transaction: &mut MutationTransaction,
) -> Result<()>;

fn execute_adoption(plan: &AdoptionPlan) -> Result<()> {
    let mut transaction = MutationTransaction::new();
    if let Err(error) = execute_adoption_plans(std::slice::from_ref(plan), &mut transaction) {
        let rollback = transaction.rollback();
        return Err(error).with_context(|| format!("rollback result: {rollback:?}"));
    }
    transaction.commit()
}
```

The aggregate executor must register every move, backup, and link in the shared `MutationTransaction`. It keeps staging backups for every plan until all canonical copies and symlinks have been created and verified. If plan N fails, reverse every completed operation from plans 1 through N. Only `MutationTransaction::commit` may delete migration backups.

`SetupPlan::execute` must create one `MutationTransaction`, create the root if needed, execute the aggregate adoption plans, execute `reconciliation` with the same transaction, verify the projected final links, then save config and commit. If config saving or reconciliation fails, roll back the entire transaction and remove the newly created root only when it is empty. Preserve the existing recovery-artifact behavior when rollback itself cannot complete.

- [ ] **Step 6: Add idempotence, dry-run, and interactive conflict tests**

Add an idempotence assertion by running the same successful setup command twice and requiring the second invocation to report no changes. Add a dry-run test proving neither config nor canonical root is created.

In `tests/interactive.rs`, rename adoption scenarios to setup and invoke:

```rust
let mut command = cargo_bin_cmd!("si");
command
    .env("HOME", &home)
    .env("SKILL_ISSUE_CONFIG", &config)
    .args(["setup", root.to_str().unwrap(), "--no-color"]);
```

Retain tests for viewing a diff, selecting a canonical version, keeping both versions, and cancelling. Update expected final text from “adopted” to “setup complete” or “No skill issues.”

Run:

```bash
cargo test --test cli setup -- --nocapture
cargo test --test interactive -- --nocapture
```

Expected: all setup and interactive tests pass.

- [ ] **Step 7: Commit the setup workflow**

```bash
git add src/setup.rs tests/interactive.rs
git add -p src/cli.rs src/lib.rs tests/cli.rs
git commit -m "feat: add guided skill repository setup"
```

---

### Task 3: Cut over to the simple public CLI and read-only status

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/lib.rs`
- Modify: `tests/cli.rs`

**Interfaces:**
- Consumes: `setup::run`, `apply::run`, existing `status_command`, `diff_command`, `targets_command`, `config_command`, and `tui::run`.
- Produces: the final eight-command Clap interface and bare-command status semantics.

- [ ] **Step 1: Replace the help contract test with the final command surface**

Replace `help_lists_the_v01_commands` with:

```rust
#[test]
fn help_lists_only_the_simple_public_workflows() {
    let output = cargo_bin_cmd!("si").arg("--help").output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for command in ["setup", "apply", "status", "tui", "diff", "targets", "config", "completions"] {
        assert!(help.contains(&format!("  {command}")), "missing {command}: {help}");
    }
    for removed in ["init", "scan", "adopt", "doctor", "link", "unlink", "enable", "disable", "delete", "restore", "sync"] {
        assert!(!help.contains(&format!("  {removed}")), "still exposes {removed}: {help}");
    }
}

#[test]
fn removed_commands_are_rejected_without_compatibility_aliases() {
    for command in ["init", "scan", "adopt", "doctor", "link", "unlink", "enable", "disable", "delete", "restore", "sync"] {
        cargo_bin_cmd!("si").arg(command).assert().failure();
    }
}
```

- [ ] **Step 2: Add a failing bare-command read-only test**

Create a divergent physical skill, run bare `si` in a non-interactive test process, assert the documented conflict exit code 3 and status text, and prove no path or file changed:

```rust
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

    assert_eq!(fs::read_to_string(target.join("foo/SKILL.md")).unwrap(), "different");
}
```

- [ ] **Step 3: Remove obsolete Clap variants and simplify dispatch**

Make `Command` contain only:

```rust
pub enum Command {
    Setup { root: Option<PathBuf> },
    Apply,
    Status,
    Tui { project: Option<PathBuf> },
    Diff { skill: String, content: bool },
    Targets { command: Option<TargetCommand> },
    Config { command: Option<ConfigCommand> },
    Completions { shell: Shell },
}
```

Remove `LinkArgs` and `UnlinkArgs`. Delete obsolete match arms and command-only orchestration functions. Keep low-level functions still consumed by setup, apply, or TUI, including toggle and delete plan construction.

Replace `default_command` with direct status dispatch:

```rust
None => match Config::load() {
    Ok(config) => {
        validate_config(&config)?;
        status_command(&config)
    }
    Err(error) if is_missing_config(&error) => {
        eprintln!("{} skill-issue is not configured. Run {}.",
            theme::warn_err(), theme::hint_err("si setup ~/code/skills"));
        Ok(EXIT_CONFIG)
    }
    Err(error) => Err(error),
},
```

Update `missing_configuration_has_documented_exit_code` to expect `si setup ~/code/skills`. Add status assertions covering the single recommendation rule: safely repairable missing links print `si apply`, divergent physical content prints `si diff foo`, and a healthy tree prints `No skill issues.` without printing another `Run:` line.

- [ ] **Step 4: Make Git status automatic and strictly read-only**

Remove the `status --git` flag and always call `git_health` inside status. Keep `git status`, `git rev-parse`, and locally known `rev-list`; do not fetch.

Delete `sync_command`, `run_git_checked`, `sync_is_current`, and `render_sync_check`. Replace their integration tests with one test that initializes a local Git repository, dirties it, runs `si status`, and asserts `repository` and `dirty` appear without modifying refs or files.

- [ ] **Step 5: Rehome overlapping TUI deletion work instead of discarding it**

Remove only the `Command::Delete` variant, `delete_skill_command`, and the three CLI delete tests currently present as uncommitted changes. Retain these crate-private interfaces for the TUI:

```rust
pub(crate) struct SkillDeletePlan {
    pub(crate) skill: String,
    canonical: PathBuf,
    links: Vec<PathBuf>,
}
pub(crate) fn plan_skill_delete(config: &Config, skill: &str) -> Result<SkillDeletePlan>;
```

Retain the existing `SkillDeletePlan::apply`, dry-run preview state, confirmation modal, and TUI tests. Do the same for `SkillTogglePlan` even though enable/disable are no longer public commands.

- [ ] **Step 6: Run the public-surface tests**

Run:

```bash
cargo test --test cli help_lists_only_the_simple_public_workflows -- --exact
cargo test --test cli removed_commands_are_rejected_without_compatibility_aliases -- --exact
cargo test --test cli bare_si_is_read_only_status -- --exact
cargo test --test cli status_recommends -- --nocapture
cargo test --test cli git_status -- --nocapture
```

Expected: all pass.

- [ ] **Step 7: Commit the CLI cutover**

```bash
git add src/cli.rs src/lib.rs tests/cli.rs
git commit -m "refactor: simplify the public CLI"
```

---

### Task 4: Align the TUI with setup/apply terminology

**Files:**
- Modify: `src/tui.rs`
- Test: `src/tui.rs`

**Interfaces:**
- Consumes: existing `SkillTogglePlan`, `SkillDeletePlan`, scan data, and theme behavior.
- Produces: unchanged TUI capabilities with guidance that names only supported commands.

- [ ] **Step 1: Add failing guidance assertions**

Update the existing unadopted-skill test to require setup vocabulary:

```rust
#[test]
fn uncollected_skill_toggle_recommends_setup() {
    let (temp, mut config) = fixture();
    fs::remove_dir_all(config.root.join("rust-cli")).unwrap();
    let physical = temp.path().join("agent/uncollected");
    fs::create_dir_all(&physical).unwrap();
    fs::write(physical.join("SKILL.md"), "body").unwrap();
    config.targets.get_mut("claude").unwrap().path = temp.path().join("agent");
    let mut app = App::new(config, false, false).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.mode, Mode::Notice));
    assert!(app.notice.contains("si setup"));
    assert!(!app.notice.contains("si adopt"));
}
```

- [ ] **Step 2: Run the TUI tests and verify the vocabulary assertion fails**

Run:

```bash
cargo test tui::tests::uncollected_skill_toggle_recommends_setup -- --exact
```

Expected: fail because the current notice recommends `si adopt`.

- [ ] **Step 3: Update guidance while retaining all current TUI actions**

Replace unadopted/not-adopted user terminology with uncollected where it improves clarity. Use:

```text
Collect with: si setup
Repair links with: si apply
```

Keep the current `d` enable/disable behavior and the uncommitted `D` permanent-delete flow. Ensure both continue using crate-level plan types rather than direct ad hoc filesystem mutation.

- [ ] **Step 4: Run and commit TUI tests**

Run:

```bash
cargo test tui::tests -- --nocapture
```

Expected: all TUI tests pass.

```bash
git add src/tui.rs src/lib.rs
git commit -m "refactor: align TUI guidance with simple workflows"
```

---

### Task 5: Rewrite the README and reconcile the PRD

**Files:**
- Modify: `README.md`
- Modify: `prd.md`
- Test: `README.md`
- Test: `prd.md`

**Interfaces:**
- Consumes: the final help output and verified setup/apply behavior.
- Produces: documentation with one canonical product promise and no removed commands.

- [ ] **Step 1: Capture the final help output before writing examples**

Run:

```bash
cargo run --quiet -- --help
cargo run --quiet -- setup --help
cargo run --quiet -- apply --help
```

Expected: help contains the final eight commands and the documented global flags. Use the actual output to avoid inventing syntax.

- [ ] **Step 2: Replace the README opening with the two-machine workflow**

The first screen of `README.md` must contain this concise promise and flow:

````markdown
`skill-issue` keeps the real copies of all your agent skills in one directory
and symlinks them into Claude Code, Codex, Gemini, `.agents`, and other configured
agents. Put that directory in Git and every computer uses the same skills.

## First computer

```bash
si setup ~/code/skills
cd ~/code/skills
git init
git add .
git commit -m "Add skills"
```

## Another computer

```bash
git clone <your-skills-repository> ~/code/skills
si setup ~/code/skills
```

## After pulling changes

```bash
git -C ~/code/skills pull
si apply
```
````

Follow it with a four-command table for `setup`, `apply`, `status`, and `tui`; a short safety section; the optional TUI screenshot/key summary; advanced commands; configuration; installation; and license.

Remove the long colour explanation, duplicated command examples, Git-wrapping sync story, and public enable/disable/delete instructions. Preserve a concise note that deletion and enable/disable remain available inside the TUI.

- [ ] **Step 3: Update the PRD to the approved product definition**

Make these exact conceptual edits:

- Product vision: “centralize every skill” rather than only “find duplicates.”
- Primary journey: `si setup ~/code/skills` performs discovery, collection, conflict resolution, and linking in one confirmed plan.
- CLI design and command lists: contain the final eight commands only.
- Replace separate init/scan/adopt/link/restore/sync command sections with setup/apply behavior, retaining fingerprinting and safety requirements.
- Machine restore story: `git clone ...` followed by `si setup ...`; ongoing changes are `git pull` followed by `si apply`.
- Git section: explicitly prohibit mutating Git commands.
- TUI: current optional interface, not a v0.3 future item.
- Acceptance tests and roadmap: use setup/apply names and behaviors.
- Core product test: one real copy in Git, symlinked to every configured agent.

Do not weaken the existing migration, conflict-preservation, filesystem-edge-case, or transactional-safety requirements.

- [ ] **Step 4: Check documentation for stale commands and broken formatting**

Run:

```bash
rg -n 'si (init|scan|adopt|doctor|link|unlink|enable|disable|delete|restore|sync)( |$|`)' README.md prd.md
rg -n 'skillissue (init|scan|adopt|doctor|link|unlink|enable|disable|delete|restore|sync)( |$|`)' README.md prd.md
git diff --check -- README.md prd.md
```

Expected: no obsolete public command examples. Historical explanation is acceptable only when explicitly marked as removed; prefer deleting it.

- [ ] **Step 5: Commit documentation**

```bash
git add README.md prd.md
git commit -m "docs: explain the simple skills workflow"
```

---

### Task 6: Complete verification and manual journey checks

**Files:**
- Modify only if verification reveals a defect in files already in scope.
- Test: all Rust unit and integration tests.

**Interfaces:**
- Consumes: the completed setup, apply, status, TUI, and documentation work.
- Produces: evidence that both user journeys work and the repository meets formatting, lint, and test requirements.

- [ ] **Step 1: Run formatting and lint checks**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: both exit 0 with no warnings.

- [ ] **Step 2: Run the complete test suite**

```bash
cargo test
```

Expected: every unit, CLI, and interactive test passes.

- [ ] **Step 3: Smoke-test first-computer setup in an isolated home**

Create a temporary directory with `mktemp -d`, record its printed path, and use that explicit path for the remainder of the smoke test. Under its `home`, create `.claude/skills/foo/SKILL.md` and `.codex/skills/foo/SKILL.md` with identical content. Set `HOME` and `SKILL_ISSUE_CONFIG` to paths inside that temporary directory, then run:

```bash
cargo run --quiet -- setup <temp>/home/code/skills --yes --no-color
cargo run --quiet -- status --no-color
```

Inspect with `find -L` and `readlink`; verify there is one real `<temp>/home/code/skills/foo` directory and both agent paths resolve to it.

- [ ] **Step 4: Smoke-test the cloned/pulled workflow without network access**

In a second explicit temporary home, pre-populate `code/skills/foo/SKILL.md`, create `.claude/skills`, configure with `si setup`, then add `code/skills/bar/SKILL.md` and run:

```bash
cargo run --quiet -- apply --yes --no-color
cargo run --quiet -- status --no-color
```

Verify both `foo` and `bar` are symlinks under `.claude/skills`. Remove the canonical `bar` directory, run `si apply --yes`, and verify the stale `.claude/skills/bar` symlink is removed without touching `foo`.

- [ ] **Step 5: Inspect the final diff for scope and accidental attribution**

```bash
git status --short
git diff --check HEAD~5..HEAD
git log -5 --format='%h %s%n%b'
rg -n 'Co-Authored-By:|Generated with|Made with|Written by' README.md prd.md src tests docs
```

Expected: only planned files changed, no whitespace errors, no agent attribution, and the pre-existing TUI deletion work remains present.

- [ ] **Step 6: Commit any verification-only fixes, if needed**

If verification required fixes, stage only the affected in-scope files and commit:

```bash
git commit -m "fix: complete simple CLI verification"
```

If no fixes were necessary, do not create an empty commit.
