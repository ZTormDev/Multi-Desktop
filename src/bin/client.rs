//! Cross-platform control client. It builds on Windows, Linux and macOS.
use multi_desktop::control::{self, AUTH_PROMPT, BANNER};
use std::{
    env,
    io::{self, BufReader, Read, Write},
    net::TcpStream,
    process::ExitCode,
    time::Duration,
};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
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
        return match pair(&address, &code) {
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
    let request = match action.as_str() {
        "INFO" | "LIST" | "PING" if id.is_none() => action,
        "PROVISION" | "STATUS" | "START" | "STOP" if id.is_some() => {
            format!("{action} {}", id.unwrap())
        }
        _ => return usage(),
    };
    match run(&address, &token, &request) {
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
        "Usage:\n  multidesktopctl <host:port> <token|--token-stdin> <INFO|LIST|PING|PROVISION|STATUS|START|STOP> [desktop-id]\n  multidesktopctl pair <host:port> <pairing-code>"
    );
    ExitCode::from(2)
}

fn pair(address: &str, code: &str) -> io::Result<String> {
    let mut stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
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
    writeln!(stream, "PAIR {code}")?;
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

fn run(address: &str, token: &str, request: &str) -> io::Result<String> {
    let mut stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
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
    writeln!(stream, "AUTH {token}")?;
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    if !line.starts_with("OK") {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            line.trim().to_owned(),
        ));
    }
    writeln!(stream, "{request}")?;
    line.clear();
    control::read_line(&mut reader, &mut line)?;
    Ok(line.trim().to_owned())
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
