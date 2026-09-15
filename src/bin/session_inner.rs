//! Runs inside the Gamescope child environment, never in the physical seat.
use std::{
    env,
    os::unix::process::CommandExt,
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    let mut command = env::args_os();
    let _program = command.next();
    let Some(program) = command.next() else {
        eprintln!("missing inner desktop command");
        return ExitCode::from(2);
    };
    let arguments: Vec<_> = command.collect();
    if let Err(error) = Command::new("/bin/sh")
        .args([
            "-c",
            "while true; do /usr/local/bin/multi-desktop-input-agent; sleep 2; done",
        ])
        .spawn()
    {
        eprintln!("could not start private input agent: {error}");
        return ExitCode::from(3);
    }
    let error = Command::new(program).args(arguments).exec();
    eprintln!("could not execute inner desktop command: {error}");
    ExitCode::from(3)
}
