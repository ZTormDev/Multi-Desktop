//! Generic isolated desktop session launcher.
use std::{
    env, fs, io,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::Path,
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    if unsafe { geteuid() } == 0 {
        eprintln!("desktop session must run as an unprivileged desktop user");
        return ExitCode::from(1);
    }
    let id = required("MULTIDESKTOP_DESKTOP_ID");
    let home = required("MULTIDESKTOP_DESKTOP_HOME");
    let runtime = required("MULTIDESKTOP_DESKTOP_RUNTIME");
    let command = required("MULTIDESKTOP_DESKTOP_COMMAND");
    if let (Some(id), Some(home), Some(runtime), Some(command)) = (id, home, runtime, command) {
        for path in [home.as_str(), runtime.as_str()] {
            if let Err(error) = prepare_directory(Path::new(path), 0o700) {
                eprintln!("cannot prepare desktop {id}: {error}");
                return ExitCode::from(2);
            }
        }
        for suffix in [".config", ".local/share", ".local/state", ".cache"] {
            if let Err(error) = prepare_directory(&Path::new(&home).join(suffix), 0o700) {
                eprintln!("cannot prepare desktop home: {error}");
                return ExitCode::from(2);
            }
        }
        let user = env::var("USER").unwrap_or_else(|_| format!("mdesk-{id}"));
        let error = Command::new("dbus-run-session")
            .args(["--", "/bin/sh", "-lc", &command])
            .env_clear()
            .env("HOME", &home)
            .env("USER", &user)
            .env("LOGNAME", &user)
            .env("SHELL", "/bin/bash")
            .env(
                "PATH",
                "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            )
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("XDG_CONFIG_HOME", format!("{home}/.config"))
            .env("XDG_DATA_HOME", format!("{home}/.local/share"))
            .env("XDG_STATE_HOME", format!("{home}/.local/state"))
            .env("XDG_CACHE_HOME", format!("{home}/.cache"))
            .env("XDG_CURRENT_DESKTOP", "MultiDesktop")
            .env("MULTIDESKTOP_SESSION", id)
            .exec();
        eprintln!("could not execute desktop compositor: {error}");
        ExitCode::from(3)
    } else {
        eprintln!("missing required Multi-Desktop session environment");
        ExitCode::from(2)
    }
}

fn required(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

fn prepare_directory(path: &Path, mode: u32) -> io::Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

unsafe extern "C" {
    fn geteuid() -> u32;
}
