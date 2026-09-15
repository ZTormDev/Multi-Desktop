use crate::config::{Config, valid_identifier};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

pub struct SessionManager {
    config: Config,
}

pub struct DoctorReport {
    pub ok: bool,
    pub items: Vec<DoctorItem>,
}

pub struct DoctorItem {
    pub name: &'static str,
    pub detail: String,
    pub ok: bool,
}

impl SessionManager {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
    pub fn token(&self) -> &str {
        &self.config.token
    }

    /// Checks prerequisites without creating users, sessions or persistent data.
    pub fn doctor(&self) -> DoctorReport {
        let mut items = vec![
            prerequisite("systemd-run", "required to isolate desktop units"),
            prerequisite("useradd", "required to provision desktop users"),
            prerequisite("dbus-run-session", "required for a private desktop bus"),
            executable(
                Path::new("/usr/local/bin/multi-desktop-session"),
                "isolated desktop session launcher",
            ),
        ];
        for program in ["gamescope", "startxfce4"] {
            if self
                .config
                .desktop_command
                .split_whitespace()
                .any(|word| word == program)
            {
                items.push(prerequisite(program, "referenced by desktop_command"));
            }
        }
        DoctorReport {
            ok: items.iter().all(|item| item.ok),
            items,
        }
    }

    pub fn list(&self) -> io::Result<String> {
        let root = PathBuf::from("/var/lib/multi-desktop/desktops");
        if !root.exists() {
            return Ok("none".to_owned());
        }
        let mut desktops = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if valid_identifier(&id) {
                desktops.push(format!("{id}:{}", self.status(&id)?));
            }
        }
        desktops.sort();
        Ok(if desktops.is_empty() {
            "none".into()
        } else {
            desktops.join(",")
        })
    }

    pub fn provision(&self, id: &str) -> io::Result<String> {
        self.validate_id(id)?;
        let user = self.user_name(id);
        let home = self.home_path(id);
        let exists = Command::new("id").arg("-u").arg(&user).status()?.success();
        if !exists {
            let status = Command::new("useradd")
                .args([
                    "--no-create-home",
                    "--home-dir",
                    home.to_str().unwrap(),
                    "--shell",
                    "/bin/bash",
                    "--groups",
                    "audio,render",
                    &user,
                ])
                .status()?;
            if !status.success() {
                return Err(io::Error::other("could not create desktop user"));
            }
        }
        fs::create_dir_all(&home)?;
        self.chown(&home, &user)?;
        Ok(format!("provisioned:{id}:{user}"))
    }

    pub fn status(&self, id: &str) -> io::Result<String> {
        self.validate_id(id)?;
        let output = Command::new("systemctl")
            .args(["is-active", &self.unit_name(id)])
            .output()?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    pub fn start(&self, id: &str) -> io::Result<String> {
        self.provision(id)?;
        if matches!(self.status(id).as_deref(), Ok("active")) {
            return Ok("already-running".into());
        }
        let user = self.user_name(id);
        let home = self.home_path(id);
        let runtime = self.runtime_path(id);
        fs::create_dir_all(&runtime)?;
        self.chown(&runtime, &user)?;
        let status = Command::new("systemd-run")
            .args([
                "--unit",
                &self.unit_name(id),
                "--collect",
                "--uid",
                &user,
                "--property=PrivateTmp=yes",
                "--property=ProtectSystem=full",
                "--property=ProtectHome=tmpfs",
                "--property=NoNewPrivileges=yes",
                "--property=RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6",
                "--setenv",
                &format!("MULTIDESKTOP_DESKTOP_ID={id}"),
                "--setenv",
                &format!("MULTIDESKTOP_DESKTOP_HOME={}", home.display()),
                "--setenv",
                &format!("MULTIDESKTOP_DESKTOP_RUNTIME={}", runtime.display()),
                "--setenv",
                &format!(
                    "MULTIDESKTOP_DESKTOP_COMMAND={}",
                    self.config.desktop_command
                ),
                "/usr/local/bin/multi-desktop-session",
            ])
            .status()?;
        if status.success() {
            Ok("started".into())
        } else {
            Err(io::Error::other("could not start desktop session"))
        }
    }

    pub fn stop(&self, id: &str) -> io::Result<String> {
        self.validate_id(id)?;
        let status = Command::new("systemctl")
            .args(["stop", &self.unit_name(id)])
            .status()?;
        if status.success() {
            Ok("stopped".into())
        } else {
            Err(io::Error::other("could not stop desktop session"))
        }
    }

    fn validate_id(&self, id: &str) -> io::Result<()> {
        if valid_identifier(id) && self.user_name(id).len() <= 32 {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "desktop id must be 1-24 ASCII letters, digits or hyphens and fit the Linux user-name limit",
            ))
        }
    }
    fn user_name(&self, id: &str) -> String {
        format!("{}{}", self.config.user_prefix, id)
    }
    fn unit_name(&self, id: &str) -> String {
        format!("multi-desktop@{id}")
    }
    fn home_path(&self, id: &str) -> PathBuf {
        PathBuf::from("/var/lib/multi-desktop/desktops")
            .join(id)
            .join("home")
    }
    fn runtime_path(&self, id: &str) -> PathBuf {
        PathBuf::from("/run/multi-desktop").join(id)
    }
    fn chown(&self, path: &Path, user: &str) -> io::Result<()> {
        let status = Command::new("chown")
            .args([
                "-R",
                "--",
                &format!("{user}:{user}"),
                path.to_str().unwrap(),
            ])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other("could not set desktop ownership"))
        }
    }
}

fn prerequisite(program: &'static str, detail: &'static str) -> DoctorItem {
    let found = env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|directory| executable(&directory.join(program), detail).ok)
    });
    DoctorItem {
        name: program,
        detail: if found {
            detail.to_owned()
        } else {
            format!("not found in PATH; {detail}")
        },
        ok: found,
    }
}

fn executable(path: &Path, detail: &'static str) -> DoctorItem {
    let ok = path.is_file()
        && fs::metadata(path)
            .map(|metadata| {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            })
            .unwrap_or(false);
    DoctorItem {
        name: if path == Path::new("/usr/local/bin/multi-desktop-session") {
            "multi-desktop-session"
        } else {
            "desktop-command"
        },
        detail: if ok {
            detail.to_owned()
        } else {
            format!("missing or not executable: {}; {detail}", path.display())
        },
        ok,
    }
}
