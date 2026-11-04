//! `dg` binary: forwards the process arguments to [`dp_cli::execute`].

use std::process::ExitCode;

fn main() -> ExitCode {
    let response = dp_cli::execute(std::env::args_os().skip(1));

    print!("{}", response.stdout);
    eprint!("{}", response.stderr);

    ExitCode::from(response.code)
}
