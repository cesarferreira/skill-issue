# Simple CLI Design

## Purpose

`skill-issue` gives a user one Git-managed directory containing the real copies
of their agent skills. Agent-specific skill directories contain symlinks into
that canonical directory.

The product supports two primary journeys:

1. On the first computer, configure a canonical directory, then use `sync` to
   collect skills scattered across installed agents and replace original copies
   with symlinks.
2. On another computer, clone or pull that canonical Git repository and apply
   its skills to the locally installed agents.

Git owns remote synchronization, history, and backup. `skill-issue` owns local
discovery, migration, symlink reconciliation, conflict detection, and
verification. It never runs mutating Git commands.

## Product language

The primary promise is not merely duplicate removal. It is:

> Keep every agent skill in one Git repository and symlink it to every agent.

Deduplication is part of the initial collection process, not the enduring
product identity. User-facing documentation should prefer the verbs “set up,”
“collect,” and “apply” over internal terms such as “adopt,” “restore,” or
“synchronize.”

## Public command surface

The public CLI contains:

```text
si setup [SKILLS_DIR]
si sync
si status
si tui
si diff <SKILL>
si targets [COMMAND]
si config [COMMAND]
si completions <SHELL>
```

The commands `init`, `scan`, `adopt`, `doctor`, `link`, `unlink`, `enable`,
`disable`, `delete`, `restore`, and `sync` are removed. There are no deprecated
aliases because the CLI is new and compatibility is not required.

Running `si` without a subcommand is exactly equivalent to `si status`. It is
always read-only and never enters an interactive migration.

The TUI remains available as an optional power-user interface. Its existing
skill inspection, agent inspection, health, enable/disable, and deletion
capabilities remain in scope.

## `si setup [SKILLS_DIR]`

`setup` is the onboarding workflow. It configures the canonical root and
discovers local agent targets; later skill changes use `sync`.

### Configuration

When no configuration exists, `setup` uses `SKILLS_DIR` when provided. In an
interactive terminal, omission prompts for the directory and defaults to
`~/skills`. Outside an interactive terminal, omission is an error.

When configuration exists, omission reuses its canonical directory. Providing
a different directory is an error that directs the user to explicitly change
the configured root first. Repeating `setup` with the configured directory is
idempotent.

Setup detects known agent directories that exist on the current machine and
merges them into the configured target set without removing custom or
previously configured targets. Machine-specific target configuration remains
outside the canonical Git repository.

### Configuration only

Setup does not move skills or create links. It stores the canonical root and
enabled target configuration, then directs the user to `si sync`. `--dry-run`
validates and previews configuration without writing it.

## `si sync`

`sync` discovers physical skills in configured agent targets, collects safe
candidates into the canonical directory, then reconciles links. It is the
command users run after installation, `git pull`, or manual changes to the
canonical directory. It does not fetch, pull, commit, or push Git data.

Sync builds a deterministic plan that may:

- create missing configured target directories;
- move a unique physical skill into the canonical directory;
- create missing links for every canonical skill in every enabled target;
- repair broken managed links when the canonical skill exists;
- replace a physical directory when its content is identical to the canonical
  skill;
- remove a stale managed symlink whose canonical skill no longer exists.

A stale managed symlink is safe to remove because only the symlink is deleted;
no physical skill content is removed.

Sync must preserve and report:

- divergent physical directories;
- foreign symlinks;
- paths whose ownership cannot be established;
- physical directories that exist only in a target and have not been collected
  because they diverge from another copy.

If any preserved conflict prevents a requested link, sync returns the
documented issues exit code and recommends `si diff <skill>`.

Sync shows its full plan before mutation, supports `--dry-run` and `--yes`,
executes transactionally, verifies every resulting link, and reports the final
health state.

## `si status` and bare `si`

Status is the everyday read-only view. It reports:

- the canonical directory;
- canonical skill count;
- enabled agent targets;
- per-agent link coverage;
- uncollected physical skills;
- missing, broken, stale, or foreign links;
- divergent content;
- interrupted-migration recovery artifacts;
- Git working-tree and upstream health when the canonical directory is itself
  a Git repository.

Git inspection is read-only. Status does not fetch, so ahead/behind information
reflects locally known remote refs.

The output ends with one actionable recommendation:

- no configuration: `si setup <skills-dir>`;
- uncollected or divergent physical skills: `si setup` or
  `si diff <skill>`;
- safely repairable link drift: `si sync`;
- healthy state: `No skill issues.`

Machine-readable JSON remains available for status and diff.

## TUI

The TUI remains an optional interface over the same domain and operation
planning code. It must not implement separate filesystem mutation rules.

Existing views and actions remain available:

- skills, agents, and health views;
- filtering and navigation;
- enable and disable;
- permanent deletion with explicit confirmation;
- dry-run behavior;
- project-local inspection where currently supported.

TUI guidance and notices use the new vocabulary. For example, an uncollected
skill recommends `si setup`, and missing canonical links recommend `si sync`.

## Internal design

The existing discovery, fingerprinting, comparison, filesystem safety,
rollback, and link-verification implementations remain the foundation.

New top-level planners separate decisions from mutation:

- `SetupPlan` aggregates the per-skill migration choices and required target
  creation/link operations.
- `ApplyPlan` represents deterministic local reconciliation, including stale
  managed-link removal.

Each workflow follows:

```text
discover -> classify -> gather decisions -> build plan -> render -> confirm
         -> execute transactionally -> verify -> rescan
```

Setup and apply planning should live in focused modules rather than adding more
orchestration to the existing large `src/lib.rs`. Shared low-level operations
remain reusable by the TUI.

Removing commands also removes their command dispatch, help text, tests, and
Git-mutating implementation. Read-only Git health remains shared by status.

## Safety and failure behavior

No workflow executes files found inside a skill.

No physical directory containing different or unclassified content is silently
deleted or overwritten. Foreign symlinks are never adopted as managed links.

Before mutation, the full operation plan is validated for path topology,
permissions where knowable, collisions, and canonical/target containment.

Setup stages physical directories before replacement. If a later operation
fails, it restores staged directories and removes links created by the failed
transaction. Sync records created, removed, and replaced links and rolls them
back if verification fails.

Non-interactive divergent content is an actionable error. `--yes` authorizes a
fully determined plan; it does not authorize conflict selection or unsafe
replacement.

## Documentation design

The README leads with the result and the two-machine workflow:

1. one-paragraph promise;
2. first-computer setup;
3. new-computer setup;
4. everyday `git pull` followed by `si sync`;
5. the four primary commands;
6. safety guarantees;
7. optional TUI;
8. advanced commands, configuration, and installation.

The README does not lead with implementation terminology, colour details, a
large command catalogue, or a key-by-key TUI manual. Those details are either
condensed or placed after the primary journey.

The PRD is updated so its command lists, primary journey, restore story,
version roadmap, and TUI status match this design.

## Verification

Automated CLI tests cover:

- setup with scattered unique and identical skills;
- setup against a pre-populated cloned canonical directory;
- safe repeated setup;
- setup conflict selection and non-interactive conflict refusal;
- apply after a canonical skill is added;
- apply after a canonical skill is removed, including stale-link cleanup;
- apply replacing an identical physical copy;
- apply preserving divergent physical directories and foreign symlinks;
- apply rollback after a later failure;
- bare `si` remaining read-only;
- missing-configuration guidance;
- removed commands being absent from help and rejected by parsing;
- read-only Git status reporting;
- existing TUI behavior and updated guidance.

The completed change must pass:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Manual smoke testing should exercise both documented journeys in isolated
temporary home directories and verify the resulting symlink topology with
filesystem inspection.

## Success criteria

A new user can understand the product from the first README screen and needs
only one command to consolidate an existing machine.

After cloning or pulling the canonical repository on another computer, the
user needs only `si setup <skills-dir>` once and `si sync` thereafter.

At rest, each skill has one real directory under the canonical root, every
enabled agent sees it through a symlink, Git remains the only remote
synchronization mechanism, and `si` reports `No skill issues.`
