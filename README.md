<div align="center">
  <h1>skillissue</h1>

  <p><strong>Deduplicate agent skills with safe canonical symlinks</strong></p>

  <p>
    <img alt="License" src="https://img.shields.io/badge/license-MIT-green">
    <img alt="Rust" src="https://img.shields.io/badge/rust-1.85%2B-orange">
    <img alt="Edition" src="https://img.shields.io/badge/edition-2024-blue">
  </p>

  <p>
    <a href="#install">Install</a>
    &nbsp;·&nbsp;
    <a href="#quickstart">Quickstart</a>
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
# Choose the directory that will hold the real copies.
si init ~/code/skills

# Inspect skills found in Claude Code, Codex, Gemini, and .agents.
si scan

# Preview the exact migration without changing anything.
si adopt --dry-run

# Run the guided migration. Destructive confirmations default to No.
si

# Verify every canonical skill and symlink.
si doctor
```

`skillissue` compares directory contents rather than timestamps. Identical
copies are moved into the canonical directory and their original locations are
replaced with symlinks. Divergent copies are never overwritten: the guided flow
asks which version to adopt or lets you keep each version under a separate
name.

Common follow-up operations:

```bash
si diff rust-cli --content
si targets add opencode ~/.config/opencode/skills
si link rust-cli claude codex
si link --all --target gemini
si unlink rust-cli codex
```

Every mutating command supports `--dry-run`. Colour is disabled by `NO_COLOR`
or `--no-color`. After reviewing a plan, `--yes` allows safe non-interactive
execution; divergent copies still require an interactive choice.

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
