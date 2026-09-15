use crate::config::{Config, valid_identifier};
use crate::pairing::{DeviceCredentials, Pairing, PairingStore};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

pub struct SessionManager {
    config: Config,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Principal {
    Admin,
    Desktop(String),
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
    pub fn authenticate(&self, token: &str) -> io::Result<Option<Principal>> {
        if token == self.config.token {
            return Ok(Some(Principal::Admin));
        }
        Ok(PairingStore::host_default()
            .desktop_for_token(token)?
            .map(Principal::Desktop))
    }

    pub fn create_pairing(&self, id: &str) -> io::Result<Pairing> {
        self.validate_id(id)?;
        PairingStore::host_default().create(id)
    }

    pub fn redeem_pairing(&self, code: &str) -> io::Result<DeviceCredentials> {
        let credentials = PairingStore::host_default().redeem(code)?;
        self.provision(&credentials.desktop_id)?;
        Ok(credentials)
    }

    pub fn revoke_devices(&self, id: &str) -> io::Result<String> {
        self.validate_id(id)?;
        let revoked = PairingStore::host_default().revoke_desktop_tokens(id)?;
        Ok(format!("revoked-devices:{revoked}"))
    }

    pub fn list_devices(&self, id: &str) -> io::Result<String> {
        self.validate_id(id)?;
        let devices = PairingStore::host_default().list_desktop_devices(id)?;
        Ok(if devices.is_empty() {
            "none".to_owned()
        } else {
            devices
                .into_iter()
                .map(|device| device.id)
                .collect::<Vec<_>>()
                .join(",")
        })
    }

    pub fn revoke_device(&self, id: &str, device_id: &str) -> io::Result<String> {
        self.validate_id(id)?;
        if PairingStore::host_default().revoke_device(id, device_id)? {
            Ok(format!("revoked-device:{device_id}"))
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "device not found"))
        }
    }

    pub fn can_manage(&self, principal: &Principal, id: &str) -> bool {
        matches!(principal, Principal::Admin)
            || matches!(principal, Principal::Desktop(owned) if owned == id)
    }

    /// Checks prerequisites without creating users, sessions or persistent data.
    pub fn doctor(&self) -> DoctorReport {
        let mut items = vec![
            prerequisite("systemd-run", "required to isolate desktop units"),
            prerequisite("useradd", "required to provision desktop users"),
            prerequisite("dbus-run-session", "required for a private desktop bus"),
            prerequisite("pipewire", "required for the private media graph"),
            prerequisite(
                "pipewire-pulse",
                "required for the private virtual audio sink",
            ),
            prerequisite("pactl", "required to create the private virtual audio sink"),
            executable(
                "multi-desktop-session",
                Path::new("/usr/local/bin/multi-desktop-session"),
                "isolated desktop session launcher",
            ),
            executable(
                "multi-desktop-capture-agent",
                Path::new("/usr/local/bin/multi-desktop-capture-agent"),
                "PipeWire capture discovery agent",
            ),
            prerequisite(
                "pw-dump",
                "required to discover the private Gamescope video node",
            ),
            executable(
                "multi-desktop-media-agent",
                Path::new("/usr/local/bin/multi-desktop-media-agent"),
                "low-latency H.264 encoder agent",
            ),
            executable(
                "multi-desktop-audio-agent",
                Path::new("/usr/local/bin/multi-desktop-audio-agent"),
                "private virtual-sink Opus encoder agent",
            ),
            executable(
                "multi-desktop-input-agent",
                Path::new("/usr/local/bin/multi-desktop-input-agent"),
                "private Gamescope/libei input agent",
            ),
            executable(
                "multi-desktop-session-inner",
                Path::new("/usr/local/bin/multi-desktop-session-inner"),
                "launcher inside the private Gamescope child environment",
            ),
            prerequisite(
                "gst-launch-1.0",
                "required to encode the private PipeWire stream",
            ),
            gst_element(
                "pipewiresrc",
                "required to consume the private capture node",
            ),
            gst_element("x264enc", "required for the baseline H.264 encoder"),
            gst_element("fdsink", "required for the private H.264 relay"),
            gst_element("pulsesrc", "required for private audio capture"),
            gst_element("opusenc", "required for Opus audio encoding"),
            gst_element("oggmux", "required for the reference Opus stream"),
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
                    "render",
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

    /// Returns stable, one-line systemd diagnostics suitable for a remote client.
    pub fn details(&self, id: &str) -> io::Result<String> {
        self.validate_id(id)?;
        let output = Command::new("systemctl")
            .args([
                "show",
                "--no-page",
                "--property=ActiveState,SubState,Result,ExecMainStatus",
                &self.unit_name(id),
            ])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other("could not inspect desktop session"));
        }
        Ok(format_systemd_details(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    pub fn capture_status(&self, id: &str) -> io::Result<String> {
        self.validate_id(id)?;
        let status =
            fs::read_to_string(self.runtime_path(id).join("capture-node")).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "capture agent has not published a node",
                )
            })?;
        Ok(status.lines().collect::<Vec<_>>().join(";"))
    }

    pub fn media_status(&self, id: &str) -> io::Result<String> {
        self.validate_id(id)?;
        Ok(
            fs::read_to_string(self.runtime_path(id).join("media-status"))?
                .lines()
                .collect::<Vec<_>>()
                .join(";"),
        )
    }

    pub fn input_status(&self, id: &str) -> io::Result<String> {
        self.validate_id(id)?;
        Ok(
            fs::read_to_string(self.runtime_path(id).join("input-status"))?
                .lines()
                .collect::<Vec<_>>()
                .join(";"),
        )
    }

    pub fn audio_status(&self, id: &str) -> io::Result<String> {
        self.validate_id(id)?;
        Ok(
            fs::read_to_string(self.runtime_path(id).join("audio-status"))?
                .lines()
                .collect::<Vec<_>>()
                .join(";"),
        )
    }

    /// Opens the session-owned input socket at a daemon-derived path. The
    /// authenticated client never learns or connects to this host-local path.
    pub fn open_input_relay(&self, id: &str) -> io::Result<UnixStream> {
        self.validate_id(id)?;
        let status = self.input_status(id)?;
        if !input_status_ready(&status) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "desktop input agent is not ready",
            ));
        }
        let socket = self.runtime_path(id).join("input-events.sock");
        validate_private_socket(&socket, "input")?;
        let stream = UnixStream::connect(socket)?;
        validate_peer_uid(&stream, self.desktop_uid(id)?)?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
        Ok(stream)
    }

    /// Opens only the daemon-derived Unix relay inside this desktop's private
    /// runtime. A remote client never receives this endpoint directly.
    pub fn open_video_relay(&self, id: &str) -> io::Result<UnixStream> {
        self.validate_id(id)?;
        let status = self.media_status(id)?;
        let mut state = None;
        let mut codec = None;
        for field in status.split(';') {
            if let Some(value) = field.strip_prefix("state=") {
                state = Some(value);
            }
            if let Some(value) = field.strip_prefix("codec=") {
                codec = Some(value);
            }
        }
        if !matches!(state, Some("encoding" | "streaming")) || codec != Some("h264") {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "desktop video encoder is not ready",
            ));
        }
        let socket = self.runtime_path(id).join("video.h264.sock");
        validate_private_socket(&socket, "video")?;
        let stream = UnixStream::connect(socket)?;
        validate_peer_uid(&stream, self.desktop_uid(id)?)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
        Ok(stream)
    }

    pub fn open_audio_relay(&self, id: &str) -> io::Result<UnixStream> {
        self.validate_id(id)?;
        let status = self.audio_status(id)?;
        let required = ["codec=opus", "container=ogg", "rate=48000", "channels=2"];
        let state_ready = status
            .split(';')
            .any(|field| matches!(field, "state=encoding" | "state=streaming"));
        if !state_ready
            || !required
                .iter()
                .all(|field| status.split(';').any(|value| value == *field))
        {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "desktop audio encoder is not ready",
            ));
        }
        let socket = self.runtime_path(id).join("audio.opus.sock");
        validate_private_socket(&socket, "audio")?;
        let stream = UnixStream::connect(socket)?;
        validate_peer_uid(&stream, self.desktop_uid(id)?)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
        Ok(stream)
    }

    pub fn start(&self, id: &str) -> io::Result<String> {
        self.provision(id)?;
        let current_state = self.status(id)?;
        if matches!(current_state.as_str(), "active" | "activating") {
            return Ok("already-running".into());
        }
        if current_state == "deactivating" {
            return Err(io::Error::other(
                "desktop session is stopping; retry shortly",
            ));
        }
        let user = self.user_name(id);
        let home = self.home_path(id);
        let runtime = self.runtime_path(id);
        if runtime.exists() {
            fs::remove_dir_all(&runtime)?;
        }
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
                "--property=Restart=on-failure",
                "--property=RestartSec=2s",
                "--property=StartLimitIntervalSec=60s",
                "--property=StartLimitBurst=3",
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
            let runtime = self.runtime_path(id);
            if runtime.exists() {
                fs::remove_dir_all(runtime)?;
            }
            Ok("stopped;runtime-cleaned".into())
        } else {
            Err(io::Error::other("could not stop desktop session"))
        }
    }

    pub fn restart(&self, id: &str) -> io::Result<String> {
        self.stop(id)?;
        self.start(id).map(|result| format!("restarted;{result}"))
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
    fn desktop_uid(&self, id: &str) -> io::Result<u32> {
        let output = Command::new("id")
            .arg("-u")
            .arg(self.user_name(id))
            .output()?;
        if !output.status.success() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "desktop user does not exist",
            ));
        }
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid desktop uid"))
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

fn format_systemd_details(output: &str) -> String {
    let details: Vec<_> = output
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| {
            matches!(
                *key,
                "ActiveState" | "SubState" | "Result" | "ExecMainStatus"
            )
        })
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    if details.is_empty() {
        "state=unknown".to_owned()
    } else {
        details.join(";")
    }
}

fn prerequisite(program: &'static str, detail: &'static str) -> DoctorItem {
    let found = env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|directory| is_executable(&directory.join(program)))
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

fn executable(name: &'static str, path: &Path, detail: &'static str) -> DoctorItem {
    let ok = is_executable(path);
    DoctorItem {
        name,
        detail: if ok {
            detail.to_owned()
        } else {
            format!("missing or not executable: {}; {detail}", path.display())
        },
        ok,
    }
}

fn gst_element(name: &'static str, detail: &'static str) -> DoctorItem {
    let ok = Command::new("gst-inspect-1.0")
        .arg(name)
        .output()
        .is_ok_and(|output| output.status.success());
    DoctorItem {
        name,
        detail: if ok {
            detail.to_owned()
        } else {
            format!("GStreamer element unavailable; {detail}")
        },
        ok,
    }
}

fn is_executable(path: &Path) -> bool {
    path.is_file()
        && fs::metadata(path)
            .map(|metadata| {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            })
            .unwrap_or(false)
}

fn is_socket(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt;
    metadata.file_type().is_socket()
}

fn validate_private_socket(path: &Path, channel: &str) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !is_socket(&metadata) {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("desktop {channel} endpoint is not a Unix socket"),
        ))
    } else {
        Ok(())
    }
}

fn validate_peer_uid(stream: &UnixStream, expected_uid: u32) -> io::Result<()> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: credentials points to writable storage of the advertised size,
    // and stream owns a valid connected Unix socket descriptor.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(credentials).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of::<libc::ucred>() || credentials.uid != expected_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "desktop relay peer has the wrong operating-system identity",
        ));
    }
    Ok(())
}

fn input_status_ready(status: &str) -> bool {
    [
        "state=ready",
        "keyboard=ready",
        "pointer=ready",
        "button=ready",
        "scroll=ready",
    ]
    .iter()
    .all(|field| status.split(';').any(|value| value == *field))
}

#[cfg(test)]
mod tests {
    use super::{
        format_systemd_details, input_status_ready, validate_peer_uid, validate_private_socket,
    };
    use std::{fs, os::unix::fs::symlink, os::unix::net::UnixListener};

    #[test]
    fn formats_only_stable_systemd_fields() {
        let details = format_systemd_details(
            "ActiveState=failed\nSubState=failed\nResult=exit-code\nExecMainStatus=127\nDescription=ignore\n",
        );
        assert_eq!(
            details,
            "ActiveState=failed;SubState=failed;Result=exit-code;ExecMainStatus=127"
        );
    }

    #[test]
    fn input_requires_every_advertised_capability() {
        assert!(input_status_ready(
            "state=ready;keyboard=ready;pointer=ready;button=ready;scroll=ready;channel=waiting"
        ));
        assert!(!input_status_ready(
            "state=ready;keyboard=ready;pointer=ready;button=ready"
        ));
    }

    #[test]
    fn private_relays_reject_regular_files_and_symlinks() {
        let root = std::env::temp_dir().join(format!(
            "multi-desktop-private-relay-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("relay.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        assert!(validate_private_socket(&socket, "test").is_ok());
        let link = root.join("relay-link.sock");
        symlink(&socket, &link).unwrap();
        assert!(validate_private_socket(&link, "test").is_err());
        let regular = root.join("regular");
        fs::write(&regular, "not a socket").unwrap();
        assert!(validate_private_socket(&regular, "test").is_err());
        drop(listener);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_relay_requires_the_expected_peer_uid() {
        let (first, _second) = std::os::unix::net::UnixStream::pair().unwrap();
        // SAFETY: geteuid has no preconditions.
        let own_uid = unsafe { libc::geteuid() };
        assert!(validate_peer_uid(&first, own_uid).is_ok());
        assert!(validate_peer_uid(&first, own_uid.wrapping_add(1)).is_err());
    }
}
