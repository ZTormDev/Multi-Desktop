use crate::session::{Principal, SessionManager};
use multi_desktop::control::{self, AUTH_PROMPT, BANNER};
use std::{
    io::{self, BufReader, Write},
    net::TcpStream,
    sync::Arc,
};

pub fn handle_client(mut stream: TcpStream, manager: Arc<SessionManager>) -> io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(15)))?;
    stream.write_all(format!("{BANNER}\n{AUTH_PROMPT}\n").as_bytes())?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let peer = stream
        .peer_addr()
        .map(|address| address.to_string())
        .unwrap_or_else(|_| "unknown".to_owned());
    let mut line = String::new();
    control::read_line(&mut reader, &mut line)?;
    let principal = if let Some(token) = line.trim().strip_prefix("AUTH ") {
        manager.authenticate(token)?
    } else if let Some(code) = line.trim().strip_prefix("PAIR ") {
        match manager.redeem_pairing(code) {
            Ok(credentials) => {
                eprintln!(
                    "audit event=pairing-redeemed peer={peer} desktop={}",
                    credentials.desktop_id
                );
                writeln!(
                    stream,
                    "OK paired desktop={} token={}",
                    credentials.desktop_id, credentials.token
                )?;
                return Ok(());
            }
            Err(error) => {
                eprintln!("audit event=pairing-failed peer={peer} detail={error}");
                stream.write_all(b"ERR pairing-failed\n")?;
                return Ok(());
            }
        }
    } else {
        None
    };
    let Some(principal) = principal else {
        eprintln!("audit event=authentication-failed peer={peer}");
        stream.write_all(b"ERR unauthorized\n")?;
        return Ok(());
    };
    stream.write_all(b"OK authenticated\n")?;
    loop {
        line.clear();
        if control::read_line(&mut reader, &mut line)? == 0 {
            return Ok(());
        }
        let mut command = line.split_whitespace();
        let verb = command.next().unwrap_or("");
        let id = command.next();
        let is_mutation = matches!(verb, "PROVISION" | "START" | "STOP");
        let response = match (verb, id, command.next()) {
            ("INFO", None, None) => Ok("version=1;features=desktop-lifecycle".to_owned()),
            ("PING", None, None) => Ok("PONG".to_owned()),
            ("LIST", None, None) if principal == Principal::Admin => manager.list(),
            ("PROVISION", Some(id), None) if principal == Principal::Admin => manager.provision(id),
            ("STATUS", Some(id), None) if manager.can_manage(&principal, id) => manager.status(id),
            ("START", Some(id), None) if manager.can_manage(&principal, id) => manager.start(id),
            ("STOP", Some(id), None) if manager.can_manage(&principal, id) => manager.stop(id),
            ("LIST" | "PROVISION" | "STATUS" | "START" | "STOP", _, _) => {
                Ok("ERR forbidden".to_owned())
            }
            ("QUIT", None, None) => return Ok(()),
            _ => Ok("ERR unknown-command".to_owned()),
        };
        match response {
            Ok(value) => {
                if is_mutation {
                    eprintln!("audit event={verb} peer={peer} result=ok detail={value}");
                }
                writeln!(stream, "OK {value}")?
            }
            Err(error) => {
                if is_mutation {
                    eprintln!("audit event={verb} peer={peer} result=error detail={error}");
                }
                writeln!(stream, "ERR {error}")?
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::{io::BufRead, net::TcpListener, thread};

    #[test]
    fn authenticates_and_reports_capabilities() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let manager = Arc::new(SessionManager::new(Config {
            listen: address.to_string(),
            token: "01234567890123456789012345678901".to_owned(),
            user_prefix: "mdesk-".to_owned(),
            desktop_command: "true".to_owned(),
        }));
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_client(stream, manager)
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
        assert_eq!(line.trim(), "OK version=1;features=desktop-lifecycle");
        writeln!(stream, "QUIT").unwrap();
        server.join().unwrap().unwrap();
    }
}
