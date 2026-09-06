<div align="center">
  <h1>skill-issue</h1>

  <p><strong>Deduplicate agent skills with safe canonical symlinks</strong></p>

  <p>Your agents have a skill issue. Fortunately, it is mostly symlinks.</p>

  <p>
    <img alt="License" src="https://img.shields.io/badge/license-MIT-green">
    <img alt="Rust" src="https://img.shields.io/badge/rust-1.85%2B-orange">
    <img alt="Edition" src="https://img.shields.io/badge/edition-2024-blue">
  </p>

  <p>
    <a href="#install">Install</a>
    &nbsp;·&nbsp;
    <a href="#quickstart">Quickstart</a>
    &nbsp;·&nbsp;
    <a href="#colour">Colour</a>
    &nbsp;·&nbsp;
    <a href="#tui">TUI</a>
  </p>
</div>

---

## Install

Requires [Rust](https://rustup.rs) **1.85+** and `~/.cargo/bin` on your `PATH`.

Download a binary from the [latest GitHub release](https://github.com/cesarferreira/skill-issue/releases/latest), or install directly from source:

```bash
cargo install --git https://github.com/cesarferreira/skill-issue.git --tag v0.2.0 --locked
```

Verify:

```bash
si --help
```

<a id="quickstart"></a>
## Quickstart

```bash
# Choose where the one true copies will live. No council required.
si init ~/code/skills

# Inspect skills found in Claude Code, Codex, Gemini, opencode, and .agents.
si scan

# Browse every skill and agent in the interactive control center.
si tui

# Preview the exact migration without changing anything.
si adopt --dry-run

# Politely ask every agent to stop hoarding.
si

# Verify every canonical skill and symlink.
si doctor
```

`skill-issue` compares directory contents rather than timestamps. Identical
copies are moved into the canonical directory and their original locations are
replaced with symlinks. Divergent copies are never overwritten: the guided flow
asks which version to adopt or lets you keep each version under a separate
name. Rejected versions are verified and preserved under
`~/.cache/skill-issue/migrations/` before their installations are linked to the
selected copy.

Once everything is healthy, `si` returns to its natural state: quietly judging
your filesystem with `✓ No skill issues.`

<a id="colour"></a>
## Colour

Every command shares one palette with the TUI, so a glance is usually enough.
`si status` reads as a dashboard, complete with per-agent coverage meters:

```text
◆ skill-issue  one true copy of every agent skill

▌ CANONICAL
  ~/skills
  18 skills
  4 agents

▌ COVERAGE
✓ Claude     18/18 ████████████
⚠ Codex      12/18 ████████░░░░
```

Cyan marks counts and paths worth reading, purple marks section headings and
preserved copies, green means healthy, yellow means repairable, and red means a
conflict that needs a decision. Plan verbs (`LINK`, `STAGE`, `REMOVE`) are
coloured by how destructive they are, and `si diff` renders like `git diff`.
Colour, including `--help`, turns off with `--no-color` or `NO_COLOR`, and can
be forced through pipes with `CLICOLOR_FORCE=1`.

<a id="tui"></a>
## Interactive TUI

Run `si tui` for a visual overview of canonical skills, installations, agent
coverage, conflicts, and broken links:

```text
 ◆ skill-issue  SKILL CONTROL CENTER
  18 skills    16 managed    1 conflict    4 agents
  SKILLS       AGENTS        HEALTH
 ┌ Skills ──────────────────────┐┌ Details ──────────────────────┐
 │ MANAGED   android-cli    4/4 ││ android-cli                   │
 │ DISABLED  old-workflow   0/4 ││ ● claude     managed link    │
 │ CONFLICT  release-notes  2/4 ││ ● codex      managed link    │
 └──────────────────────────────┘└───────────────────────────────┘
```

- `↑`/`↓` or `j`/`k` navigates; `/` filters skills.
- `Tab` switches between Skills, Agents, and Health views.
- `d` previews and confirms enable/disable for the selected canonical skill.
- `r` rescans the filesystem; `?` opens the complete keyboard guide.
- `si tui --project .` includes project-local `.claude` and `.agents` skills.
- `--dry-run` keeps action previews fully read-only, and `--no-color` uses a
  monochrome theme.

Disabling removes only verified, managed symlinks from every configured agent.
The canonical skill is never deleted. Re-enable it from the TUI or with
`si enable <skill>`. The equivalent non-interactive preview is
`si disable <skill> --dry-run`.

Common follow-up operations:

```bash
si diff rust-cli --content
si targets add opencode ~/.config/opencode/skills
si link rust-cli --all
si link --all
si link rust-cli --target gemini
si unlink rust-cli --target gemini
si disable rust-cli
si enable rust-cli
```

Every mutating command supports `--dry-run`. Colour is disabled by `NO_COLOR`
or `--no-color` and forced by `CLICOLOR_FORCE=1`. After reviewing a plan,
`--yes` allows safe non-interactive
execution; divergent copies still require an interactive choice. With a skill
name, `link --all` links that skill into every detected agent. Without a skill
name, it links every canonical skill into every detected agent.

Configuration lives at `~/.config/skill-issue/config.toml`. Set
`SKILL_ISSUE_CONFIG` to use another location. Existing installations using the
legacy path or environment variable continue to work.

## More workflows

Include project-local Claude and `.agents` skills, optionally as JSON:

```bash
si scan --project .
si scan --project . --json
```

Inspect Git backup health and restore a freshly cloned canonical directory:

```bash
si status --git
si restore --dry-run
si restore
```

## Multiple computers

Keep the canonical skill directory in a Git repository and clone it at the same
configured root on each computer. Each machine can use different local agent
targets; `si sync` updates the canonical checkout and repairs the links for that
machine:

```bash
# Fetch remote state and report Git or symlink drift without changing skills.
si sync --check

# Fast-forward the canonical repository, then recreate missing agent links.
si sync
```

`si sync --check` exits successfully only when the working tree is clean, the
branch is neither ahead nor behind its upstream, and all configured agent links
are healthy. Use `si sync --check --json` in scripts or a status line.

Sync is deliberately conservative: it refuses dirty or diverged repositories,
pulls with `--ff-only`, never pushes local commits, and never replaces physical
directories or foreign symlinks. Review the operation without fetching or
changing anything with `si sync --dry-run`.

Configure portable relative links and exclude generated content from
fingerprints:

```bash
si config set-relative-links true
si config add-ignore '*.log'
si config remove-ignore '*.log'
```

Generate completions for Bash, Zsh, Fish, Elvish, or PowerShell:

```bash
si completions zsh > ~/.zfunc/_si
```

## License

MIT
