//! Encodes the private Gamescope PipeWire node into a session-local Unix relay.
use std::{
    env, fs, io,
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::UnixListener,
    },
    path::Path,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

fn main() -> io::Result<()> {
    let runtime = env::var("MULTIDESKTOP_DESKTOP_RUNTIME")
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "missing desktop runtime"))?;
    let runtime = Path::new(&runtime);
    let capture = runtime.join("capture-node");
    let status = runtime.join("media-status");
    let socket = runtime.join("video.h264.sock");
    loop {
        let Some(node) = ready_node(&capture) else {
            write_status(&status, "state=waiting-for-capture\n")?;
            thread::sleep(Duration::from_secs(1));
            continue;
        };
        remove_stale_socket(&socket)?;
        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        write_status(
            &status,
            &format!("state=encoding\nnode_id={node}\ncodec=h264\nrelay=private-unix\n"),
        )?;
        let (mut relay, _) = listener.accept()?;
        let mut encoder = Command::new("gst-launch-1.0")
            .args([
                "-q",
                "pipewiresrc",
                &format!("path={node}"),
                "!",
                "queue",
                "!",
                "videoconvert",
                "!",
                "x264enc",
                "tune=zerolatency",
                "speed-preset=ultrafast",
                "bitrate=6000",
                "key-int-max=60",
                "!",
                "video/x-h264,profile=baseline",
                "!",
                "h264parse",
                "config-interval=1",
                "!",
                "video/x-h264,stream-format=byte-stream,alignment=au",
                "!",
                "fdsink",
                "fd=1",
                "sync=false",
            ])
            .stdout(Stdio::piped())
            .spawn()?;
        write_status(
            &status,
            &format!("state=streaming\nnode_id={node}\ncodec=h264\nrelay=private-unix\n"),
        )?;
        let copy_result = io::copy(
            encoder
                .stdout
                .as_mut()
                .ok_or_else(|| io::Error::other("encoder stdout is unavailable"))?,
            &mut relay,
        );
        let _ = encoder.kill();
        let result = encoder.wait()?;
        write_status(
            &status,
            &format!(
                "state=encoder-exited\nexit_code={}\n",
                result.code().unwrap_or(-1)
            ),
        )?;
        if let Err(error) = copy_result {
            write_status(
                &status,
                &format!(
                    "state=relay-ended\nreason={}\n",
                    sanitize(&error.to_string())
                ),
            )?;
        }
        thread::sleep(Duration::from_secs(2));
    }
}
fn ready_node(path: &Path) -> Option<u64> {
    let contents = fs::read_to_string(path).ok()?;
    contents
        .lines()
        .any(|line| line == "state=ready")
        .then(|| {
            contents
                .lines()
                .find_map(|line| line.strip_prefix("node_id=")?.parse().ok())
        })
        .flatten()
}
fn write_status(path: &Path, contents: &str) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(temporary, path)
}

fn remove_stale_socket(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "video relay path exists and is not a socket",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character != '\n' && *character != '\r')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::remove_stale_socket;
    use std::{fs, os::unix::net::UnixListener};

    #[test]
    fn removes_only_a_stale_unix_socket() {
        let root = std::env::temp_dir().join(format!(
            "multi-desktop-media-socket-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("relay.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        drop(listener);
        remove_stale_socket(&socket).unwrap();
        assert!(!socket.exists());
        fs::write(&socket, "do-not-remove").unwrap();
        assert!(remove_stale_socket(&socket).is_err());
        assert!(socket.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
