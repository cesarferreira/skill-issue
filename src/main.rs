use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    match skill_issue::run(skill_issue::Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
