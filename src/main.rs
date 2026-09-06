use std::process::ExitCode;

fn main() -> ExitCode {
    match skill_issue::run(skill_issue::parse_cli()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{} {error:#}", skill_issue::error_prefix());
            ExitCode::FAILURE
        }
    }
}
