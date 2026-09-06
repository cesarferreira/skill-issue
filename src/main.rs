use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    match skillissue::run(skillissue::Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
