//! Encodes the private Gamescope PipeWire node as low-latency H.264 shared memory.
use std::{
    env, fs, io, os::unix::fs::PermissionsExt, path::Path, process::Command, thread, time::Duration,
};

fn main() -> io::Result<()> {
    let runtime = env::var("MULTIDESKTOP_DESKTOP_RUNTIME")
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "missing desktop runtime"))?;
    let desktop_id = env::var("MULTIDESKTOP_SESSION")
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "missing desktop id"))?;
    let port = media_port(&desktop_id);
    let runtime = Path::new(&runtime);
    let capture = runtime.join("capture-node");
    let status = runtime.join("media-status");
    let socket = runtime.join("video.h264.shm");
    loop {
        let Some(node) = ready_node(&capture) else {
            write_status(&status, "state=waiting-for-capture\n")?;
            thread::sleep(Duration::from_secs(1));
            continue;
        };
        let _ = fs::remove_file(&socket);
        write_status(
            &status,
            &format!(
                "state=encoding\nnode_id={node}\ncodec=h264\nendpoint={}\nrelay=127.0.0.1:{port}\n",
                socket.display(),
            ),
        )?;
        let result = Command::new("gst-launch-1.0")
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
                "tee",
                "name=video",
                "video.",
                "!",
                "queue",
                "!",
                "shmsink",
                &format!("socket-path={}", socket.display()),
                "wait-for-connection=false",
                "sync=false",
                "video.",
                "!",
                "queue",
                "!",
                "tcpserversink",
                "host=127.0.0.1",
                &format!("port={port}"),
                "sync=false",
            ])
            .status()?;
        write_status(
            &status,
            &format!(
                "state=encoder-exited\nexit_code={}\n",
                result.code().unwrap_or(-1)
            ),
        )?;
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

fn media_port(desktop_id: &str) -> u16 {
    let hash = desktop_id.bytes().fold(2_166_136_261_u32, |state, byte| {
        (state ^ u32::from(byte)).wrapping_mul(16_777_619)
    });
    49_000 + (hash % 1_000) as u16
}

#[cfg(test)]
mod tests {
    use super::media_port;

    #[test]
    fn assigns_a_stable_loopback_relay_port() {
        let port = media_port("laptop");
        assert!((49_000..50_000).contains(&port));
        assert_eq!(port, media_port("laptop"));
    }
}
