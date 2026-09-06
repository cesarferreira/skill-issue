# skill-issue

Keep the real copies of all your agent skills in one directory, then symlink
them into Claude Code, Codex, Gemini, `.agents`, and any other configured
agent. Put that directory in Git and every computer uses the same skills.

Your agents have a skill issue. Fortunately, it is mostly symlinks.

## First computer

`setup` finds the skills already installed in your agents, moves one copy of
each into a canonical directory, and links every detected agent back to it.
It compares content, never timestamps, and asks before resolving a conflict.

```bash
si setup ~/code/skills

cd ~/code/skills
git init
git add .
git commit -m "Add skills"
git remote add origin <your-skills-repository>
git push -u origin main
```

## Another computer

Clone your skills repository, then let `setup` detect the local agents and
create their links.

```bash
git clone <your-skills-repository> ~/code/skills
si setup ~/code/skills
```

If a skill source lives outside a standard agent directory, include it during
the first setup. Ignore names only when you intentionally do not want them
collected:

```bash
si setup ~/skills \
  --target dotfiles=~/dotfiles/agent-skills/.agents/skills \
  --ignore retired-skill
```

## After pulling changes

Git remains responsible for synchronization. `si apply` only reconciles local
agent links: it adds links for new skills, repairs managed links, and removes
stale managed links for skills deleted through Git.

```bash
git -C ~/code/skills pull
si apply
```

## Everyday commands

| Command | What it does |
| --- | --- |
| `si setup [PATH]` | Configure a canonical directory, collect local skills, and link detected agents. |
| `si apply` | Reconcile canonical skills into every configured agent. Run after `git pull`. |
| `si status` or `si` | Read-only overview of skills, agent coverage, conflicts, and Git health. |
| `si tui` | Optional interactive dashboard for inspecting and managing skills. |

Useful advanced commands are `si diff <skill>`, `si targets`, and `si config`.

## Safety

- A skill has one real directory in the canonical root; agent directories use symlinks.
- `--dry-run` previews every mutating command without changing files.
- Identical physical copies can be safely replaced with links.
- Different copies and foreign symlinks are preserved and reported.
- `si` never executes skill contents and never runs `git pull`, commits, or pushes.

## TUI

Run `si tui` for a visual dashboard of skills, agents, coverage, conflicts, and
broken links. It keeps the power-user controls for enabling, disabling, and
deleting a canonical skill, each with a preview and confirmation.

Use `↑`/`↓` or `j`/`k` to navigate, `Tab` to switch views, `/` to filter,
`d` to enable or disable, `D` to delete, `r` to rescan, and `?` for help.

## Configuration

Configuration is local to the machine at `~/.config/skill-issue/config.toml`.
The canonical directory is deliberately separate so it can be your Git
repository, while each computer can have a different set of agent targets.

Add a non-standard target with:

```bash
si targets add opencode ~/.config/opencode/skills
```

## Install

Requires Rust 1.85+.

```bash
cargo install --git https://github.com/cesarferreira/skill-issue.git --locked
si --help
```

## License

MIT
