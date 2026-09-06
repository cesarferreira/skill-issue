# Home Skills Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Support a safe `~/skills` migration through `si setup`, including Cursor and first-run custom targets/ignores.

**Architecture:** Extend Clap's setup arguments with persisted setup inputs. Keep configuration merging in the existing setup/configuration path, before scanning, so the established adoption and apply planners retain ownership of filesystem changes.

**Tech Stack:** Rust 2024, clap, serde/TOML, assert_cmd, tempfile.

**Spec:** `docs/superpowers/specs/2026-09-06-home-skills-migration-design.md`

## Global Constraints

- Never overwrite divergent physical skill directories.
- `fleet-cli` and `android-cli` are excluded by explicit ignore patterns.
- Provider-managed foreign symlinks stay untouched.
- All mutating CLI paths retain `--dry-run` semantics.

---

### Task 1: Model first-run migration inputs

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/lib.rs`
- Test: `tests/cli.rs`

**Interfaces:**
- Produces `Command::Setup { root, targets, ignore }` where `targets: Vec<String>` holds `ID=PATH` values and `ignore: Vec<String>` holds glob patterns.
- Consumes `Config { targets, ignore, .. }` from `src/model.rs`.

- [ ] **Step 1: Write failing CLI coverage**

```rust
#[test]
fn setup_accepts_custom_targets_and_ignores_before_collection() {
    // fixture contains a target-only skill and an ignored skill
    // `si setup ROOT --target extra=PATH --ignore retired --yes` collects only target-only
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --test cli setup_accepts_custom_targets_and_ignores_before_collection`

Expected: FAIL because setup does not accept `--target` or `--ignore`.

- [ ] **Step 3: Add setup arguments and configuration merge**

```rust
Setup {
    root: Option<PathBuf>,
    #[arg(long = "target", value_name = "ID=PATH")]
    targets: Vec<String>,
    #[arg(long = "ignore", value_name = "PATTERN")]
    ignore: Vec<String>,
}
```

Parse target values once, reject malformed or conflicting IDs, validate ignore
globs with `build_ignore_set`, and save the merged `Config` before `scan`.
When `--dry-run` is set, render the initial configuration plan but do not save.

- [ ] **Step 4: Run focused tests**

Run: `cargo test --test cli setup_accepts_custom_targets_and_ignores_before_collection`

Expected: PASS; canonical root contains the custom-target skill, ignores the
retired skill, and configuration records both inputs.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/lib.rs tests/cli.rs
git commit -m "feat: configure setup targets and ignores"
```

### Task 2: Discover Cursor skills

**Files:**
- Modify: `src/lib.rs`
- Test: `tests/cli.rs`

**Interfaces:**
- Produces a `cursor` `TargetConfig` for `~/.cursor/skills` when it exists.

- [ ] **Step 1: Write a failing Cursor fixture test**

```rust
#[test]
fn setup_discovers_cursor_skills() {
    // create HOME/.cursor/skills/foo; setup ROOT; assert foo is linked
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --test cli setup_discovers_cursor_skills`

Expected: FAIL because Cursor is absent from built-in discovery.

- [ ] **Step 3: Add Cursor to the built-in target table**

```rust
("cursor", ".cursor/skills"),
```

Place it with the other home-relative target definitions in `init` and maintain
the existing deterministic target ranking for presentation.

- [ ] **Step 4: Run focused tests**

Run: `cargo test --test cli setup_discovers_cursor_skills`

Expected: PASS; Cursor skill is canonicalized and replaced by a managed link.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs tests/cli.rs
git commit -m "feat: discover Cursor skills"
```

### Task 3: Document and validate the migration path

**Files:**
- Modify: `README.md`
- Test: `tests/cli.rs`

**Interfaces:**
- Documents `si setup ~/skills --target ID=PATH --ignore PATTERN` as the way to
  seed a canonical root from non-standard paths.

- [ ] **Step 1: Add a README example**

```markdown
si setup ~/skills --target legacy=~/legacy/skills --ignore retired-skill
```

- [ ] **Step 2: Run full verification**

Run: `cargo fmt --check && cargo test && cargo clippy -- -D warnings`

Expected: all commands exit 0.

- [ ] **Step 3: Commit**

```bash
git add README.md tests/cli.rs
git commit -m "docs: explain setup migration options"
```

### Task 4: Execute the user migration

**Files:**
- Create: timestamped archive outside the repository
- Modify: `/Users/cesarferreira/skills` and supplied agent skill roots

**Interfaces:**
- Consumes the verified `si setup` command from Tasks 1-3.
- Produces one canonical physical copy in `~/skills` per collected skill and
  managed links at configured targets.

- [ ] **Step 1: Archive the five source roots**

Run a timestamped `tar` archive before any deletion or move and list its
contents to confirm all five roots are present.

- [ ] **Step 2: Remove explicitly retired skills**

Remove only named `fleet-cli` and `android-cli` entries after inspecting each
path and recording whether it is a link or real directory.

- [ ] **Step 3: Preview the aggregate plan**

```bash
si setup ~/skills --target dotfiles=~/dotfiles/agent-skills/.agents/skills \
  --ignore fleet-cli --ignore android-cli --dry-run
```

Expected: no retired skill is collected; divergent or foreign entries remain
listed for review.

- [ ] **Step 4: Apply and verify**

Run the same command with `--yes`, then `si status --no-color`. Confirm every
managed link resolves inside `~/skills`, no retired entries remain in supplied
roots, and the archive remains readable.

- [ ] **Step 5: Commit code documentation only**

```bash
git add README.md src tests docs
git commit -m "feat: support home skills migration"
```
