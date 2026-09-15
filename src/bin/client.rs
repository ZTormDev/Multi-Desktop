//! Cross-platform control client. It builds on Windows, Linux and macOS.
use multi_desktop::control::{self, AUTH_PROMPT, BANNER};
use multi_desktop::tls;
use multi_desktop::transport::{Frame, FrameKind, InputEvent};
use rustls::{ClientConnection, StreamOwned, pki_types::ServerName};
use std::{
    env,
    fs::OpenOptions,
    io::{self, BufReader, Read, Write},
    net::TcpStream,
    path::Path,
    process::{Command, ExitCode, Stdio},
    time::Duration,
};

enum ControlStream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Read for ControlStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for ControlStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1).peekable();
    let tls_ca = if args.next_if(|arg| arg == "--tls-ca").is_some() {
        let Some(path) = args.next() else {
            return usage();
        };
        Some(path)
    } else {
        None
    };
    let tls_server_name = if tls_ca.is_some() {
        if args.next_if(|arg| arg == "--server-name").is_some() {
            let Some(name) = args.next() else {
                return usage();
            };
            Some(name)
        } else {
            None
        }
    } else {
        None
    };
    let Some(address) = args.next() else {
        return usage();
    };
    if address == "pair" {
        let Some(address) = args.next() else {
            return usage();
        };
        let Some(code) = args.next() else {
            return usage();
        };
        if args.next().is_some() {
            return usage();
        }
        return match pair(
            &address,
            &code,
            tls_ca.as_deref(),
            tls_server_name.as_deref(),
        ) {
            Ok(response) => {
                println!("{response}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("pairing failed: {error}");
                ExitCode::from(1)
            }
        };
    }
    let Some(token_argument) = args.next() else {
        return usage();
    };
    let token = if token_argument == "--token-stdin" {
        match token_from_stdin() {
            Ok(token) => token,
            Err(error) => {
                eprintln!("could not read token: {error}");
                return ExitCode::from(2);
            }
        }
    } else {
        token_argument
    };
    let Some(action) = args.next() else {
        return usage();
    };
    let id = args.next();
    if matches!(
        action.as_str(),
        "INPUT-KEY" | "INPUT-MOVE" | "INPUT-CLICK" | "INPUT-SCROLL"
    ) {
        let Some(id) = id else {
            return usage();
        };
        let events = match action.as_str() {
            "INPUT-KEY" => {
                let (Some(code), None) = (args.next(), args.next()) else {
                    return usage();
                };
                let Ok(code) = parse_u16(&code) else {
                    return usage();
                };
                vec![
                    InputEvent::Key {
                        code,
                        pressed: true,
                    },
                    InputEvent::Key {
                        code,
                        pressed: false,
                    },
                ]
            }
            "INPUT-MOVE" | "INPUT-SCROLL" => {
                let (Some(x), Some(y), None) = (args.next(), args.next(), args.next()) else {
                    return usage();
                };
                let (Ok(x), Ok(y)) = (x.parse::<i16>(), y.parse::<i16>()) else {
                    return usage();
                };
                if action == "INPUT-MOVE" {
                    vec![InputEvent::PointerMotion { x, y }]
                } else {
                    vec![InputEvent::Scroll { x, y }]
                }
            }
            "INPUT-CLICK" => {
                let (Some(button), None) = (args.next(), args.next()) else {
                    return usage();
                };
                let code = match button.as_str() {
                    "left" => 0x110,
                    "right" => 0x111,
                    "middle" => 0x112,
                    _ => return usage(),
                };
                vec![
                    InputEvent::PointerButton {
                        code,
                        pressed: true,
                    },
                    InputEvent::PointerButton {
                        code,
                        pressed: false,
                    },
                ]
            }
            _ => unreachable!(),
        };
        return match send_input(
            &address,
            &token,
            &id,
            &events,
            tls_ca.as_deref(),
            tls_server_name.as_deref(),
        ) {
            Ok(()) => {
                println!("sent {} input event(s) to {id}", events.len());
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("input failed: {error}");
                ExitCode::from(1)
            }
        };
    }
    if action == "VIDEO" {
        let (Some(id), Some(output)) = (id, args.next()) else {
            return usage();
        };
        if args.next().is_some() {
            return usage();
        }
        return match save_video(
            &address,
            &token,
            &id,
            Path::new(&output),
            tls_ca.as_deref(),
            tls_server_name.as_deref(),
        ) {
            Ok(bytes) => {
                println!("saved {bytes} H.264 bytes to {output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("video stream failed: {error}");
                ExitCode::from(1)
            }
        };
    }
    if action == "AUDIO" {
        let (Some(id), Some(output)) = (id, args.next()) else {
            return usage();
        };
        if args.next().is_some() {
            return usage();
        }
        return match save_audio(
            &address,
            &token,
            &id,
            Path::new(&output),
            tls_ca.as_deref(),
            tls_server_name.as_deref(),
        ) {
            Ok(bytes) => {
                println!("saved {bytes} Ogg Opus bytes to {output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("audio stream failed: {error}");
                ExitCode::from(1)
            }
        };
    }
    if action == "WATCH" {
        let Some(id) = id else {
            return usage();
        };
        if args.next().is_some() {
            return usage();
        }
        return match watch_video(
            &address,
            &token,
            &id,
            tls_ca.as_deref(),
            tls_server_name.as_deref(),
        ) {
            Ok(bytes) => {
                println!("viewer ended after receiving {bytes} H.264 bytes");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("live viewer failed: {error}");
                ExitCode::from(1)
            }
        };
    }
    if action == "LISTEN" {
        let Some(id) = id else {
            return usage();
        };
        if args.next().is_some() {
            return usage();
        }
        return match listen_audio(
            &address,
            &token,
            &id,
            tls_ca.as_deref(),
            tls_server_name.as_deref(),
        ) {
            Ok(bytes) => {
                println!("audio player ended after receiving {bytes} Ogg Opus bytes");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("audio playback failed: {error}");
                ExitCode::from(1)
            }
        };
    }
    let extra = args.next();
    let request = match action.as_str() {
        "INFO" | "LIST" | "PING" if id.is_none() => action,
        "PROVISION" | "DEVICES" | "REVOKE" | "STATUS" | "DETAILS" | "CAPTURE" | "MEDIA"
        | "AUDIO-STATUS" | "INPUT" | "START" | "RESTART" | "STOP"
            if id.is_some() && extra.is_none() =>
        {
            format!("{action} {}", id.unwrap())
        }
        "REVOKE-DEVICE" if id.is_some() && extra.is_some() && args.next().is_none() => {
            format!("{action} {} {}", id.unwrap(), extra.unwrap())
        }
        _ => return usage(),
    };
    match run(
        &address,
        &token,
        &request,
        tls_ca.as_deref(),
        tls_server_name.as_deref(),
    ) {
        Ok(response) => {
            println!("{response}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("connection failed: {error}");
            ExitCode::from(1)
        }
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "Usage:\n  multidesktopctl [--tls-ca <ca.pem> [--server-name <name>]] <host:port> <token|--token-stdin> <INFO|LIST|PING|PROVISION|DEVICES|REVOKE|STATUS|DETAILS|CAPTURE|MEDIA|AUDIO-STATUS|INPUT|START|RESTART|STOP> [desktop-id]\n  multidesktopctl [--tls-ca <ca.pem> [--server-name <name>]] <host:port> <token|--token-stdin> REVOKE-DEVICE <desktop-id> <device-id>\n  multidesktopctl [--tls-ca <ca.pem> [--server-name <name>]] <host:port> <token|--token-stdin> <INPUT-KEY desktop-id evdev-code|INPUT-MOVE desktop-id x y|INPUT-CLICK desktop-id left|right|middle|INPUT-SCROLL desktop-id x y>\n  multidesktopctl [--tls-ca <ca.pem> [--server-name <name>]] <host:port> <token|--token-stdin> <VIDEO desktop-id output.h264|AUDIO desktop-id output.ogg>\n  multidesktopctl [--tls-ca <ca.pem> [--server-name <name>]] <host:port> <token|--token-stdin> <WATCH|LISTEN> <desktop-id>\n  multidesktopctl [--tls-ca <ca.pem> [--server-name <name>]] pair <host:port> <pairing-code>"
    );
    ExitCode::from(2)
}

fn pair(
    address: &str,
    code: &str,
    tls_ca: Option<&str>,
    server_name: Option<&str>,
) -> io::Result<String> {
    let stream = connect(address, tls_ca, server_name)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    control::read_line(&mut reader, &mut line)?;
    if line.trim() != BANNER {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incompatible server",
        ));
    }
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    if line.trim() != AUTH_PROMPT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid pairing prompt",
        ));
    }
    writeln!(reader.get_mut(), "PAIR {code}")?;
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    if !line.starts_with("OK paired ") {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            line.trim().to_owned(),
        ));
    }
    Ok(line.trim().to_owned())
}

fn run(
    address: &str,
    token: &str,
    request: &str,
    tls_ca: Option<&str>,
    server_name: Option<&str>,
) -> io::Result<String> {
    let stream = connect(address, tls_ca, server_name)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    control::read_line(&mut reader, &mut line)?;
    if line.trim() != BANNER {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "server is not a compatible Multi-Desktop control endpoint",
        ));
    }
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    if line.trim() != AUTH_PROMPT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "server sent an invalid authentication prompt",
        ));
    }
    writeln!(reader.get_mut(), "AUTH {token}")?;
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    if !line.starts_with("OK") {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            line.trim().to_owned(),
        ));
    }
    writeln!(reader.get_mut(), "{request}")?;
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    command_response(&line)
}

fn command_response(line: &str) -> io::Result<String> {
    if !line.starts_with("OK") {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            line.trim().to_owned(),
        ));
    }
    Ok(line.trim().to_owned())
}

fn save_video(
    address: &str,
    token: &str,
    desktop_id: &str,
    output: &Path,
    tls_ca: Option<&str>,
    server_name: Option<&str>,
) -> io::Result<u64> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    receive_video(address, token, desktop_id, &mut output, tls_ca, server_name)
}

fn watch_video(
    address: &str,
    token: &str,
    desktop_id: &str,
    tls_ca: Option<&str>,
    server_name: Option<&str>,
) -> io::Result<u64> {
    let mut player = Command::new("ffplay")
        .args([
            "-loglevel",
            "warning",
            "-fflags",
            "nobuffer",
            "-flags",
            "low_delay",
            "-framedrop",
            "-f",
            "h264",
            "-i",
            "-",
        ])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not start ffplay; install FFmpeg first: {error}"),
            )
        })?;
    let result = receive_video(
        address,
        token,
        desktop_id,
        player
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::other("ffplay stdin is unavailable"))?,
        tls_ca,
        server_name,
    );
    drop(player.stdin.take());
    let _ = player.wait();
    result
}

fn save_audio(
    address: &str,
    token: &str,
    desktop_id: &str,
    output: &Path,
    tls_ca: Option<&str>,
    server_name: Option<&str>,
) -> io::Result<u64> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    receive_audio(address, token, desktop_id, &mut output, tls_ca, server_name)
}

fn listen_audio(
    address: &str,
    token: &str,
    desktop_id: &str,
    tls_ca: Option<&str>,
    server_name: Option<&str>,
) -> io::Result<u64> {
    let mut player = Command::new("ffplay")
        .args([
            "-nodisp",
            "-autoexit",
            "-loglevel",
            "warning",
            "-fflags",
            "nobuffer",
            "-f",
            "ogg",
            "-i",
            "-",
        ])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not start ffplay; install FFmpeg first: {error}"),
            )
        })?;
    let result = receive_audio(
        address,
        token,
        desktop_id,
        player
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::other("ffplay stdin is unavailable"))?,
        tls_ca,
        server_name,
    );
    drop(player.stdin.take());
    let _ = player.wait();
    result
}

fn receive_video(
    address: &str,
    token: &str,
    desktop_id: &str,
    output: &mut impl Write,
    tls_ca: Option<&str>,
    server_name: Option<&str>,
) -> io::Result<u64> {
    let stream = connect(address, tls_ca, server_name)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    control::read_line(&mut reader, &mut line)?;
    if line.trim() != BANNER {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incompatible server",
        ));
    }
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    if line.trim() != AUTH_PROMPT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid authentication prompt",
        ));
    }
    writeln!(reader.get_mut(), "AUTH {token}")?;
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    if !line.starts_with("OK") {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            line.trim().to_owned(),
        ));
    }
    writeln!(reader.get_mut(), "VIDEO {desktop_id}")?;
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    if !line.starts_with("OK video;codec=h264;framing=annex-b") {
        return Err(io::Error::other(line.trim().to_owned()));
    }
    let config = multi_desktop::transport::Frame::read_from(&mut reader)?;
    if config.kind != multi_desktop::transport::FrameKind::VideoConfig
        || config.payload != b"codec=h264;framing=annex-b"
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected video configuration",
        ));
    }
    let mut bytes = 0_u64;
    loop {
        let frame = match multi_desktop::transport::Frame::read_from(&mut reader) {
            Ok(frame) => frame,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(bytes),
            Err(error) => return Err(error),
        };
        if frame.kind != multi_desktop::transport::FrameKind::VideoData {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected video frame",
            ));
        }
        output.write_all(&frame.payload)?;
        bytes += frame.payload.len() as u64;
    }
}

fn receive_audio(
    address: &str,
    token: &str,
    desktop_id: &str,
    output: &mut impl Write,
    tls_ca: Option<&str>,
    server_name: Option<&str>,
) -> io::Result<u64> {
    let stream = connect(address, tls_ca, server_name)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    control::read_line(&mut reader, &mut line)?;
    if line.trim() != BANNER {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incompatible server",
        ));
    }
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    if line.trim() != AUTH_PROMPT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid authentication prompt",
        ));
    }
    writeln!(reader.get_mut(), "AUTH {token}")?;
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    command_response(&line)?;
    writeln!(reader.get_mut(), "AUDIO {desktop_id}")?;
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    if line.trim() != "OK audio;codec=opus;container=ogg;rate=48000;channels=2" {
        return Err(io::Error::other(line.trim().to_owned()));
    }
    let configuration = b"codec=opus;container=ogg;rate=48000;channels=2";
    let config = Frame::read_from(&mut reader)?;
    if config.kind != FrameKind::AudioConfig || config.payload != configuration {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected audio configuration",
        ));
    }
    let mut bytes = 0_u64;
    loop {
        let frame = match Frame::read_from(&mut reader) {
            Ok(frame) => frame,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(bytes),
            Err(error) => return Err(error),
        };
        if frame.kind != FrameKind::AudioData {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected audio frame",
            ));
        }
        output.write_all(&frame.payload)?;
        bytes += frame.payload.len() as u64;
    }
}

fn send_input(
    address: &str,
    token: &str,
    desktop_id: &str,
    events: &[InputEvent],
    tls_ca: Option<&str>,
    server_name: Option<&str>,
) -> io::Result<()> {
    for event in events {
        InputEvent::decode(&event.encode())?;
    }
    let stream = connect(address, tls_ca, server_name)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    control::read_line(&mut reader, &mut line)?;
    if line.trim() != BANNER {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incompatible server",
        ));
    }
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    if line.trim() != AUTH_PROMPT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid authentication prompt",
        ));
    }
    writeln!(reader.get_mut(), "AUTH {token}")?;
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    command_response(&line)?;
    writeln!(reader.get_mut(), "INPUT-STREAM {desktop_id}")?;
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    if line.trim() != "OK input;framing=mdes;events=key,pointer,button,scroll" {
        return Err(io::Error::other(line.trim().to_owned()));
    }
    for event in events {
        Frame {
            kind: FrameKind::Input,
            payload: event.encode(),
        }
        .write_to(reader.get_mut())?;
    }
    Frame {
        kind: FrameKind::Input,
        payload: InputEvent::Reset.encode(),
    }
    .write_to(reader.get_mut())?;
    reader.get_mut().flush()
}

fn parse_u16(value: &str) -> Result<u16, std::num::ParseIntError> {
    if let Some(hex) = value.strip_prefix("0x") {
        u16::from_str_radix(hex, 16)
    } else {
        value.parse()
    }
}

fn connect(
    address: &str,
    tls_ca: Option<&str>,
    server_name: Option<&str>,
) -> io::Result<ControlStream> {
    let stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    let Some(ca_path) = tls_ca else {
        return Ok(ControlStream::Plain(stream));
    };
    let name = server_name.unwrap_or_else(|| address.split(':').next().unwrap_or(address));
    let name = ServerName::try_from(name.to_owned())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid TLS server name"))?;
    let connection = ClientConnection::new(tls::load_client_config(Path::new(ca_path))?, name)
        .map_err(io::Error::other)?;
    Ok(ControlStream::Tls(Box::new(StreamOwned::new(
        connection, stream,
    ))))
}

fn token_from_stdin() -> io::Result<String> {
    let mut token = String::new();
    io::stdin().read_to_string(&mut token)?;
    let token = token.trim().to_owned();
    if token.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "token input is empty",
        ))
    } else {
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::{command_response, parse_u16};

    #[test]
    fn command_errors_produce_a_failing_client_result() {
        assert_eq!(command_response("OK PONG\n").unwrap(), "OK PONG");
        let error = command_response("ERR forbidden\n").unwrap_err();
        assert_eq!(error.to_string(), "ERR forbidden");
    }

    #[test]
    fn parses_decimal_and_hexadecimal_evdev_codes() {
        assert_eq!(parse_u16("30").unwrap(), 30);
        assert_eq!(parse_u16("0x110").unwrap(), 0x110);
        assert!(parse_u16("nope").is_err());
    }
}
