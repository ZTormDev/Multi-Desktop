use crate::session::{Principal, SessionManager};
use multi_desktop::{
    control::{self, AUTH_PROMPT, BANNER},
    transport::{Frame, FrameKind, InputEvent},
};
use std::{
    collections::{HashMap, VecDeque},
    io::{self, BufReader, Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const MAX_AUTH_FAILURES: usize = 5;
const AUTH_FAILURE_WINDOW: Duration = Duration::from_secs(60);

pub struct AuthRateLimiter {
    attempts: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl AuthRateLimiter {
    pub fn new() -> Self {
        Self {
            attempts: Mutex::new(HashMap::new()),
        }
    }

    fn limited(&self, peer: &str, now: Instant) -> bool {
        let mut attempts = self.attempts.lock().expect("rate limiter mutex poisoned");
        let entries = attempts.entry(peer.to_owned()).or_default();
        prune(entries, now);
        entries.len() >= MAX_AUTH_FAILURES
    }

    fn record_failure(&self, peer: &str, now: Instant) {
        let mut attempts = self.attempts.lock().expect("rate limiter mutex poisoned");
        let entries = attempts.entry(peer.to_owned()).or_default();
        prune(entries, now);
        entries.push_back(now);
    }

    fn clear(&self, peer: &str) {
        self.attempts
            .lock()
            .expect("rate limiter mutex poisoned")
            .remove(peer);
    }
}

fn prune(entries: &mut VecDeque<Instant>, now: Instant) {
    while entries
        .front()
        .is_some_and(|attempt| now.duration_since(*attempt) >= AUTH_FAILURE_WINDOW)
    {
        entries.pop_front();
    }
}

pub fn handle_client<S: Read + Write>(
    stream: S,
    manager: Arc<SessionManager>,
    peer: String,
    limiter: Arc<AuthRateLimiter>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    reader
        .get_mut()
        .write_all(format!("{BANNER}\n{AUTH_PROMPT}\n").as_bytes())?;
    if limiter.limited(&peer, Instant::now()) {
        eprintln!("audit event=authentication-rate-limited peer={peer}");
        reader.get_mut().write_all(b"ERR rate-limited\n")?;
        return Ok(());
    }
    let mut line = String::new();
    control::read_line(&mut reader, &mut line)?;
    let principal = if let Some(token) = line.trim().strip_prefix("AUTH ") {
        manager.authenticate(token)?
    } else if let Some(code) = line.trim().strip_prefix("PAIR ") {
        match manager.redeem_pairing(code) {
            Ok(credentials) => {
                limiter.clear(&peer);
                eprintln!(
                    "audit event=pairing-redeemed peer={peer} desktop={}",
                    credentials.desktop_id
                );
                writeln!(
                    reader.get_mut(),
                    "OK paired desktop={} token={}",
                    credentials.desktop_id,
                    credentials.token
                )?;
                return Ok(());
            }
            Err(error) => {
                limiter.record_failure(&peer, Instant::now());
                eprintln!("audit event=pairing-failed peer={peer} detail={error}");
                reader.get_mut().write_all(b"ERR pairing-failed\n")?;
                return Ok(());
            }
        }
    } else {
        None
    };
    let Some(principal) = principal else {
        limiter.record_failure(&peer, Instant::now());
        eprintln!("audit event=authentication-failed peer={peer}");
        reader.get_mut().write_all(b"ERR unauthorized\n")?;
        return Ok(());
    };
    limiter.clear(&peer);
    reader.get_mut().write_all(b"OK authenticated\n")?;
    loop {
        line.clear();
        if control::read_line(&mut reader, &mut line)? == 0 {
            return Ok(());
        }
        let mut command = line.split_whitespace();
        let verb = command.next().unwrap_or("");
        let id = command.next();
        let argument = command.next();
        if matches!(verb, "VIDEO" | "AUDIO" | "INPUT-STREAM") {
            let response = match (id, argument) {
                (Some(id), None) if manager.can_manage(&principal, id) => {
                    if verb == "VIDEO" {
                        stream_video(id, &manager, reader.get_mut())
                    } else if verb == "AUDIO" {
                        stream_audio(id, &manager, reader.get_mut())
                    } else {
                        stream_input(id, &manager, &mut reader)
                    }
                }
                (Some(_), None) => {
                    Err(io::Error::new(io::ErrorKind::PermissionDenied, "forbidden"))
                }
                _ => {
                    reader.get_mut().write_all(b"ERR forbidden\n")?;
                    continue;
                }
            };
            match response {
                Ok(()) => return Ok(()),
                Err(error) => {
                    eprintln!("audit event=VIDEO peer={peer} result=error detail={error}");
                    writeln!(reader.get_mut(), "ERR {error}")?;
                    continue;
                }
            }
        }
        let is_mutation = matches!(
            verb,
            "PROVISION" | "REVOKE" | "REVOKE-DEVICE" | "START" | "RESTART" | "STOP"
        );
        let response = match (verb, id, argument, command.next()) {
            ("INFO", None, None, None) => Ok(
                "version=1;features=desktop-lifecycle,device-management,private-h264-capture,private-opus-audio,private-input"
                    .to_owned(),
            ),
            ("PING", None, None, None) => Ok("PONG".to_owned()),
            ("LIST", None, None, None) if principal == Principal::Admin => manager.list(),
            ("PROVISION", Some(id), None, None) if principal == Principal::Admin => {
                manager.provision(id)
            }
            ("DEVICES", Some(id), None, None) if principal == Principal::Admin => {
                manager.list_devices(id)
            }
            ("REVOKE", Some(id), None, None) if principal == Principal::Admin => {
                manager.revoke_devices(id)
            }
            ("REVOKE-DEVICE", Some(id), Some(device_id), None) if principal == Principal::Admin => {
                manager.revoke_device(id, device_id)
            }
            ("STATUS", Some(id), None, None) if manager.can_manage(&principal, id) => {
                manager.status(id)
            }
            ("DETAILS", Some(id), None, None) if manager.can_manage(&principal, id) => {
                manager.details(id)
            }
            ("CAPTURE", Some(id), None, None) if manager.can_manage(&principal, id) => {
                manager.capture_status(id)
            }
            ("MEDIA", Some(id), None, None) if manager.can_manage(&principal, id) => {
                manager.media_status(id)
            }
            ("AUDIO-STATUS", Some(id), None, None) if manager.can_manage(&principal, id) => {
                manager.audio_status(id)
            }
            ("INPUT", Some(id), None, None) if manager.can_manage(&principal, id) => {
                manager.input_status(id)
            }
            ("START", Some(id), None, None) if manager.can_manage(&principal, id) => {
                manager.start(id)
            }
            ("RESTART", Some(id), None, None) if manager.can_manage(&principal, id) => {
                manager.restart(id)
            }
            ("STOP", Some(id), None, None) if manager.can_manage(&principal, id) => {
                manager.stop(id)
            }
            (
                "LIST" | "PROVISION" | "DEVICES" | "REVOKE" | "REVOKE-DEVICE" | "STATUS"
                | "DETAILS" | "CAPTURE" | "MEDIA" | "AUDIO-STATUS" | "AUDIO" | "INPUT"
                | "INPUT-STREAM" | "VIDEO" | "START" | "RESTART" | "STOP",
                _,
                _,
                _,
            ) => Err(io::Error::new(io::ErrorKind::PermissionDenied, "forbidden")),
            ("QUIT", None, None, None) => return Ok(()),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown-command",
            )),
        };
        match response {
            Ok(value) => {
                if is_mutation {
                    eprintln!("audit event={verb} peer={peer} result=ok detail={value}");
                }
                writeln!(reader.get_mut(), "OK {value}")?
            }
            Err(error) => {
                if is_mutation {
                    eprintln!("audit event={verb} peer={peer} result=error detail={error}");
                }
                writeln!(reader.get_mut(), "ERR {error}")?
            }
        }
    }
}

fn stream_video<S: Write>(id: &str, manager: &SessionManager, client: &mut S) -> io::Result<()> {
    let mut relay = manager.open_video_relay(id)?;
    client.write_all(b"OK video;codec=h264;framing=annex-b\n")?;
    Frame {
        kind: FrameKind::VideoConfig,
        payload: b"codec=h264;framing=annex-b".to_vec(),
    }
    .write_to(&mut *client)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = relay.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        Frame {
            kind: FrameKind::VideoData,
            payload: buffer[..count].to_vec(),
        }
        .write_to(&mut *client)?;
    }
}

fn stream_audio<S: Write>(id: &str, manager: &SessionManager, client: &mut S) -> io::Result<()> {
    let mut relay = manager.open_audio_relay(id)?;
    let configuration = b"codec=opus;container=ogg;rate=48000;channels=2";
    client.write_all(b"OK audio;codec=opus;container=ogg;rate=48000;channels=2\n")?;
    Frame {
        kind: FrameKind::AudioConfig,
        payload: configuration.to_vec(),
    }
    .write_to(&mut *client)?;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = relay.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        Frame {
            kind: FrameKind::AudioData,
            payload: buffer[..count].to_vec(),
        }
        .write_to(&mut *client)?;
    }
}

fn stream_input<S: Read + Write>(
    id: &str,
    manager: &SessionManager,
    client: &mut BufReader<S>,
) -> io::Result<()> {
    let mut relay = manager.open_input_relay(id)?;
    client
        .get_mut()
        .write_all(b"OK input;framing=mdes;events=key,pointer,button,scroll\n")?;
    forward_input_frames(client, &mut relay)
}

fn forward_input_frames<R: Read, W: Write>(client: &mut R, relay: &mut W) -> io::Result<()> {
    let result = loop {
        let frame = match Frame::read_from_with_limit(&mut *client, InputEvent::MAX_ENCODED_LEN) {
            Ok(frame) => frame,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break Ok(()),
            Err(error) => break Err(error),
        };
        if frame.kind != FrameKind::Input {
            break Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected frame on input channel",
            ));
        }
        let event = match InputEvent::decode(&frame.payload) {
            Ok(event) => event,
            Err(error) => break Err(error),
        };
        if let Err(error) = relay.write_all(&event.encode()) {
            break Err(error);
        }
    };
    // The agent interprets reset and EOF as a focus loss and releases every
    // held key/button before ending this emulation transaction.
    let _ = relay.write_all(&InputEvent::Reset.encode());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::{
        io::BufRead,
        net::{TcpListener, TcpStream},
        thread,
    };

    #[test]
    fn authenticates_and_reports_capabilities() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let manager = Arc::new(SessionManager::new(Config {
            listen: address.to_string(),
            token: "01234567890123456789012345678901".to_owned(),
            user_prefix: "mdesk-".to_owned(),
            desktop_command: "true".to_owned(),
            tls_certificate: None,
            tls_private_key: None,
        }));
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_client(
                stream,
                manager,
                "test".to_owned(),
                Arc::new(AuthRateLimiter::new()),
            )
        });

        let mut stream = TcpStream::connect(address).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), BANNER);
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), AUTH_PROMPT);
        writeln!(stream, "AUTH 01234567890123456789012345678901").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "OK authenticated");
        writeln!(stream, "INFO").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(
            line.trim(),
            "OK version=1;features=desktop-lifecycle,device-management,private-h264-capture,private-opus-audio,private-input"
        );
        writeln!(stream, "NOT-A-COMMAND").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "ERR unknown-command");
        writeln!(stream, "QUIT").unwrap();
        server.join().unwrap().unwrap();
    }

    #[test]
    fn rate_limiter_blocks_the_sixth_failure_inside_one_minute() {
        let limiter = AuthRateLimiter::new();
        let now = Instant::now();
        for _ in 0..MAX_AUTH_FAILURES {
            assert!(!limiter.limited("peer", now));
            limiter.record_failure("peer", now);
        }
        assert!(limiter.limited("peer", now));
        limiter.clear("peer");
        assert!(!limiter.limited("peer", now));
    }

    #[test]
    fn forwards_only_valid_input_frames_and_appends_reset() {
        let events = [
            InputEvent::Key {
                code: 30,
                pressed: true,
            },
            InputEvent::PointerMotion { x: 12, y: -4 },
        ];
        let mut network = Vec::new();
        for event in events {
            Frame {
                kind: FrameKind::Input,
                payload: event.encode(),
            }
            .write_to(&mut network)
            .unwrap();
        }
        let mut relay = Vec::new();
        forward_input_frames(&mut network.as_slice(), &mut relay).unwrap();
        let expected: Vec<_> = events
            .into_iter()
            .flat_map(InputEvent::encode)
            .chain(InputEvent::Reset.encode())
            .collect();
        assert_eq!(relay, expected);
    }

    #[test]
    fn rejects_non_input_frames_and_still_resets_the_agent() {
        let mut network = Vec::new();
        Frame {
            kind: FrameKind::VideoData,
            payload: Vec::new(),
        }
        .write_to(&mut network)
        .unwrap();
        let mut relay = Vec::new();
        assert!(forward_input_frames(&mut network.as_slice(), &mut relay).is_err());
        assert_eq!(relay, InputEvent::Reset.encode());
    }
}
