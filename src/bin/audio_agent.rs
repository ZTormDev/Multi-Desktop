//! Captures only the virtual sink monitor in this desktop's private Pulse graph.
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

const SINK_NAME: &str = "multi_desktop_audio";

fn main() -> io::Result<()> {
    let runtime = env::var("MULTIDESKTOP_DESKTOP_RUNTIME")
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "missing desktop runtime"))?;
    let runtime = Path::new(&runtime);
    let status = runtime.join("audio-status");
    let socket = runtime.join("audio.opus.sock");
    loop {
        if let Err(error) = ensure_virtual_sink() {
            write_status(
                &status,
                &format!(
                    "state=waiting-for-sink\nreason={}\n",
                    sanitize(&error.to_string())
                ),
            )?;
            thread::sleep(Duration::from_secs(1));
            continue;
        }
        remove_stale_socket(&socket)?;
        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        write_status(
            &status,
            "state=encoding\ncodec=opus\ncontainer=ogg\nrate=48000\nchannels=2\nrelay=private-unix\n",
        )?;
        let (mut relay, _) = listener.accept()?;
        let mut encoder = Command::new("gst-launch-1.0")
            .args([
                "-q",
                "pulsesrc",
                "device=multi_desktop_audio.monitor",
                "!",
                "audioconvert",
                "!",
                "audioresample",
                "!",
                "audio/x-raw,rate=48000,channels=2",
                "!",
                "opusenc",
                "bitrate=128000",
                "frame-size=20",
                "audio-type=restricted-lowdelay",
                "!",
                "oggmux",
                "!",
                "fdsink",
                "fd=1",
                "sync=false",
            ])
            .stdout(Stdio::piped())
            .spawn()?;
        write_status(
            &status,
            "state=streaming\ncodec=opus\ncontainer=ogg\nrate=48000\nchannels=2\nrelay=private-unix\n",
        )?;
        let copy_result = io::copy(
            encoder
                .stdout
                .as_mut()
                .ok_or_else(|| io::Error::other("audio encoder stdout is unavailable"))?,
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
        thread::sleep(Duration::from_secs(1));
    }
}

fn ensure_virtual_sink() -> io::Result<()> {
    let output = Command::new("pactl")
        .args(["list", "short", "sinks"])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("private Pulse server is not ready"));
    }
    let sinks = String::from_utf8_lossy(&output.stdout);
    if sinks
        .lines()
        .any(|line| line.split_whitespace().nth(1) == Some(SINK_NAME))
    {
        return Ok(());
    }
    let status = Command::new("pactl")
        .args([
            "load-module",
            "module-null-sink",
            "sink_name=multi_desktop_audio",
            "sink_properties=device.description=Multi-Desktop",
            "format=s16le",
            "rate=48000",
            "channels=2",
            "channel_map=front-left,front-right",
        ])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("could not create private virtual sink"))
    }
}

fn remove_stale_socket(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "audio relay path exists and is not a socket",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn write_status(path: &Path, contents: &str) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(temporary, path)
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character != '\n' && *character != '\r')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::SINK_NAME;

    #[test]
    fn recognizes_only_the_named_private_sink() {
        let sinks = "12\tother\tPipeWire\n37\tmulti_desktop_audio\tPipeWire\n";
        assert!(
            sinks
                .lines()
                .any(|line| line.split_whitespace().nth(1) == Some(SINK_NAME))
        );
        let sinks = "12\tphysical-output\tPipeWire\n";
        assert!(
            !sinks
                .lines()
                .any(|line| line.split_whitespace().nth(1) == Some(SINK_NAME))
        );
    }
}
