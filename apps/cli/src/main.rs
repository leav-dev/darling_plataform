//! `dg` binary: forwards the process arguments and environment to
//! [`dp_cli::execute`].

use std::process::ExitCode;

fn main() -> ExitCode {
    let env = dp_cli::CliEnv::from_process();
    let response = dp_cli::execute(std::env::args_os().skip(1), &env);

    print!("{}", response.stdout);
    eprint!("{}", response.stderr);

    ExitCode::from(response.code)
}
