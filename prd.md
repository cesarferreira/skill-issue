# skill-issue PRD

## Product

`skill-issue` keeps the real copies of a developer’s AI-agent skills in one
canonical directory and exposes them to locally installed agents through
symlinks.

The canonical directory is intended to be a Git repository. Git handles backup,
history, branches, and synchronization between computers. `skill-issue` handles
local discovery, safe collection, linking, reconciliation, and verification.

The filesystem is the database. Git is the backup. Symlinks are the local
synchronization protocol.

## User problem

Skills often accumulate independently in paths such as:

```text
~/.claude/skills/rust-cli
~/.codex/skills/rust-cli
~/.gemini/skills/rust-cli
~/.agents/skills/rust-cli
```

Those copies drift. Developers do not know which one is canonical, cannot
reliably update every agent, and have no simple way to move their skills to a
new computer.

The desired steady state is:

```text
~/code/skills/rust-cli/                 real directory, tracked by Git
~/.claude/skills/rust-cli               symlink -> ~/code/skills/rust-cli
~/.codex/skills/rust-cli                symlink -> ~/code/skills/rust-cli
```

Every compatible agent sees the same skill. There is exactly one physical copy.

## Target user

Developers on macOS or Linux who use multiple coding agents, understand basic
Git and symlink concepts, and want personal skills to follow them between
machines.

## Principles

1. Local first: no account, server, registry, database, or background sync.
2. Safe before clever: never silently overwrite different content.
3. Git owns remote synchronization: the CLI never fetches, pulls, commits, or
   pushes.
4. Human-readable state: managed installations are ordinary symlinks.
5. One obvious path: the primary CLI should describe user goals, not filesystem
   implementation details.

## Primary workflows

### First computer

```bash
si setup ~/code/skills
si sync
cd ~/code/skills
git init
git add .
git commit -m "Add skills"
```

`setup` configures the canonical root and detects existing agent skill
locations. `sync` compares physical skill directories, moves safe candidates
into the root, replaces copies with links, and links canonical skills into
every detected agent target.

### New computer

```bash
git clone <your-skills-repository> ~/code/skills
si setup ~/code/skills
si sync
```

When the canonical directory already contains skills, `sync` treats it as the
authority and creates local links for detected agents.

### Everyday updates

```bash
git -C ~/code/skills pull
si sync
```

`sync` collects unambiguous physical skill directories into the canonical
directory, then reconciles it into configured agent targets. It creates missing
links, repairs managed links, replaces identical physical copies, and removes
stale managed links. It does not collect divergent local content and does not
run Git.

## Public CLI

```text
si setup [SKILLS_DIR]
si sync
si status
si tui
si diff <SKILL>
si targets [COMMAND]
si config [COMMAND]
```

Bare `si` is exactly `si status`. It is always read-only.

`tui` remains an optional power-user interface for navigating skills, agents,
and health. It keeps enable/disable and permanent-delete controls behind
preview and confirmation flows.

## Safety requirements

- All mutating commands support `--dry-run` and render their planned changes.
- Content equivalence uses a deterministic directory fingerprint, not timestamps.
- Different physical copies require a user decision; non-interactive conflicts
  fail without mutation.
- A foreign symlink is preserved and reported.
- A physical directory is replaced only when its fingerprint equals the
  canonical skill’s fingerprint.
- A stale managed link may be removed only when it clearly points directly into
  the canonical root and its canonical target no longer exists.
- Operations verify newly created links and roll back link changes if a later
  link operation fails.
- Skills are untrusted filesystem data. The CLI never executes their scripts.

## Agent targets

Built-in discovery covers existing skill directories for Claude Code, Codex,
Gemini, generic `.agents`, OpenCode, and Hermes. Users can add arbitrary
targets:

```bash
si targets add opencode ~/.config/opencode/skills
```

Target configuration is machine-local at `~/.config/skill-issue/config.toml`.
It does not belong inside the canonical repository because each computer may
have different agents installed.

## Status and diagnostics

`si status` reports the canonical root, skill count, agent coverage, physical
copies, conflicts, missing or broken links, and interrupted migration artifacts.
When the root is a Git repository it also reports read-only working-tree and
upstream health based on local Git refs.

`si diff <skill>` compares divergent copies. Status should recommend one next
step: `si setup` for uncollected content, `si sync` for repairable link drift,
or `si diff <skill>` for a conflict.

## Non-goals

`skill-issue` is not a package manager, marketplace, registry, cloud sync
service, agent manager, plugin manager, MCP manager, prompt generator, or Git
wrapper.

## Acceptance criteria

Given identical copies under Claude, Codex, and Gemini, `si sync` produces one
real canonical directory and symlinks at all three original paths. All original
files remain accessible through their links.

Given a cloned canonical repository and installed local agents, `si sync`
links every canonical skill into each detected target without moving canonical
content.

Given a later `git pull` that adds or removes skills, `si sync` creates the
corresponding links and removes only stale managed links.

Given divergent physical content or a foreign symlink, `si sync` leaves it
unchanged and exits with an actionable issue report.

When every enabled agent points to canonical skills, `si status` ends with:

```text
✓ No skill issues.
```
