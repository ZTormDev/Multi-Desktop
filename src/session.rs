use crate::config::{Config, valid_identifier};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

pub struct SessionManager {
    config: Config,
}

impl SessionManager {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
    pub fn token(&self) -> &str {
        &self.config.token
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
