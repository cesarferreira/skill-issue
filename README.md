# skill-issue

Keep the real copies of all your agent skills in one directory, then symlink
them into Claude Code, Codex, Gemini, `.agents`, and any other configured
agent. Put that directory in Git and every computer uses the same skills.

Your agents have a skill issue. Fortunately, it is mostly symlinks.

## First computer

`setup` configures the canonical directory and detects installed agents. `sync`
then collects local skills into that directory and reconciles every agent link.
It compares content, never timestamps, and asks before resolving a conflict.

```bash
si setup ~/code/skills
si sync

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
si sync
```

If a skill source lives outside a standard agent directory, include it during
the first setup. Ignore names only when you intentionally do not want them
collected:

```bash
si setup ~/skills \
  --target dotfiles=~/dotfiles/agent-skills/.agents/skills \
  --ignore retired-skill
si sync
```

## After pulling changes

Git remains responsible for remote synchronization. `si sync` only reconciles local
agent links: it adds links for new skills, repairs managed links, and removes
stale managed links for skills deleted through Git.

```bash
git -C ~/code/skills pull
si sync
```

## Everyday changes

`si sync` reconciles links after any change to the canonical directory, whether
the change came from Git or from you editing it directly.

Delete a canonical skill, then remove its stale managed links everywhere:

```bash
rm -rf ~/skills/old-skill
si sync
```

Add new skill directories to `~/skills` and run the same command. It creates
their links in every configured agent while removing links for any skills you
deleted:

```bash
si sync
```

If a new skill is installed directly into an agent directory, collect it with
`sync`. It shows a plan, moves the physical copy into the canonical directory,
and links it back to its original agent directory:

```bash
si sync
```

## Everyday commands

| Command | What it does |
| --- | --- |
| `si setup [PATH]` | Configure a canonical directory and detect local agent targets. |
| `si sync` | Collect local skills and reconcile canonical skills into every configured agent. |
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
