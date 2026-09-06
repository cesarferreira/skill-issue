use clap::builder::styling::{Ansi256Color, Color, Style, Styles};
use clap::{ArgAction, Args, ColorChoice, CommandFactory, FromArgMatches, Parser, Subcommand};
use clap_complete::Shell;
use std::path::PathBuf;

const fn ansi(value: u8) -> Option<Color> {
    Some(Color::Ansi256(Ansi256Color(value)))
}

/// Match `--help` to the palette the commands themselves print in.
const HELP_STYLES: Styles = Styles::styled()
    .header(Style::new().bold().fg_color(ansi(141)))
    .usage(Style::new().bold().fg_color(ansi(141)))
    .literal(Style::new().bold().fg_color(ansi(117)))
    .placeholder(Style::new().fg_color(ansi(245)))
    .valid(Style::new().fg_color(ansi(114)))
    .invalid(Style::new().bold().fg_color(ansi(221)))
    .error(Style::new().bold().fg_color(ansi(210)));

#[derive(Parser, Debug)]
#[command(
    name = "si",
    version,
    about = "Deduplicate agent skills with safe canonical symlinks",
    styles = HELP_STYLES
)]
pub struct Cli {
    /// Print the exact operation plan without changing files.
    #[arg(long, global = true)]
    pub dry_run: bool,
    /// Disable coloured output.
    #[arg(long, global = true)]
    pub no_color: bool,
    /// Increase diagnostic detail (-v or -vv).
    #[arg(short, action = ArgAction::Count, global = true)]
    pub verbose: u8,
    /// Emit machine-readable JSON for read commands.
    #[arg(long, global = true)]
    pub json: bool,
    /// Apply a displayed safe plan without prompting.
    #[arg(long, global = true)]
    pub yes: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Open the interactive skill dashboard.
    Tui {
        /// Include project-local .claude and .agents skills.
        #[arg(long, value_name = "PATH", num_args = 0..=1, default_missing_value = ".")]
        project: Option<PathBuf>,
    },
    /// Configure the canonical skill directory.
    Init { root: Option<PathBuf> },
    /// Scan configured skill locations without changing them.
    Scan {
        /// Include project-local .claude and .agents skills.
        #[arg(long, value_name = "PATH", num_args = 0..=1, default_missing_value = ".")]
        project: Option<PathBuf>,
    },
    /// Move skills into the canonical root and replace copies with links.
    Adopt { skill: Option<String> },
    /// Show the current health summary.
    Status {
        /// Include canonical-root Git health.
        #[arg(long)]
        git: bool,
    },
    /// Diagnose and optionally repair unambiguous issues.
    Doctor {
        #[arg(long)]
        fix: bool,
    },
    /// Compare the distinct copies of a skill.
    Diff {
        skill: String,
        #[arg(long)]
        content: bool,
    },
    /// Expose canonical skills to configured targets.
    Link(LinkArgs),
    /// Remove managed links without removing canonical skills.
    Unlink(UnlinkArgs),
    /// Hide a canonical skill from every configured agent.
    Disable { skill: String },
    /// Expose a canonical skill to every configured agent.
    Enable { skill: String },
    /// Permanently delete a canonical skill and every link to it.
    Delete { skill: String },
    /// List or edit target directories.
    Targets {
        #[command(subcommand)]
        command: Option<TargetCommand>,
    },
    /// Show or update configuration.
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommand>,
    },
    /// Link every canonical skill into every configured target.
    Restore,
    /// Pull the canonical Git repository and repair agent links.
    Sync {
        /// Fetch and report Git/link drift without changing skills or links.
        #[arg(long)]
        check: bool,
    },
    /// Generate shell completion definitions.
    Completions { shell: Shell },
}

#[derive(Args, Debug)]
pub struct LinkArgs {
    pub skill: Option<String>,
    pub targets: Vec<String>,
    /// Use all canonical skills, or all targets when a skill is supplied.
    #[arg(long)]
    pub all: bool,
    #[arg(long = "target", action = ArgAction::Append)]
    pub target: Vec<String>,
}

#[derive(Args, Debug)]
pub struct UnlinkArgs {
    pub skill: String,
    pub targets: Vec<String>,
    #[arg(long = "target", action = ArgAction::Append)]
    pub target: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum TargetCommand {
    Add { id: String, path: PathBuf },
    Remove { id: String },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    SetRoot {
        root: PathBuf,
    },
    SetRelativeLinks {
        #[arg(action = ArgAction::Set)]
        enabled: bool,
    },
    AddIgnore {
        pattern: String,
    },
    RemoveIgnore {
        pattern: String,
    },
}

pub fn command() -> clap::Command {
    Cli::command()
}

/// Parse arguments so `--no-color` also silences clap's own help and errors.
pub fn parse() -> Cli {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let mut command = Cli::command();
    if colour_is_unwanted(&args) {
        command = command.color(ColorChoice::Never);
    }
    let matches = command.get_matches_from(args);
    match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    }
}

fn colour_is_unwanted(args: &[std::ffi::OsString]) -> bool {
    std::env::var_os("NO_COLOR").is_some() || args.iter().any(|arg| arg == "--no-color")
}
