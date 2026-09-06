use clap::{ArgAction, Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "skillissue",
    version,
    about = "Deduplicate agent skills with safe canonical symlinks"
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
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Configure the canonical skill directory.
    Init { root: Option<PathBuf> },
    /// Scan configured skill locations without changing them.
    Scan,
    /// Move skills into the canonical root and replace copies with links.
    Adopt { skill: Option<String> },
    /// Show the current health summary.
    Status,
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
    Unlink { skill: String, targets: Vec<String> },
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
}

#[derive(Args, Debug)]
pub struct LinkArgs {
    pub skill: Option<String>,
    pub targets: Vec<String>,
    #[arg(long, conflicts_with = "skill")]
    pub all: bool,
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
    SetRoot { root: PathBuf },
}
