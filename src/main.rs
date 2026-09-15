mod config;
mod pairing;
mod protocol;
mod session;

use std::{env, net::TcpListener, path::Path, process::ExitCode, sync::Arc, thread};

use config::Config;
use multi_desktop::tls;
use protocol::{AuthRateLimiter, handle_client};
use rustls::{ServerConnection, StreamOwned};
use session::SessionManager;

const DEFAULT_CONFIG: &str = "/etc/multi-desktop/multi-desktop.conf";

fn usage() {
    eprintln!(
        "Usage:\n  multidesktopd serve [config-path]\n  multidesktopd check [config-path]\n  multidesktopd doctor [config-path]\n  multidesktopd pair <desktop-id> [config-path]\n  multidesktopd tls-check <certificate.pem> <private-key.pem>\n\nThe daemon must run as root. It provisions and supervises isolated desktop sessions."
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("serve");
    let config_path = if command == "pair" {
        args.get(3).map(String::as_str).unwrap_or(DEFAULT_CONFIG)
    } else {
        args.get(2).map(String::as_str).unwrap_or(DEFAULT_CONFIG)
    };

    if command == "--help" || command == "help" {
        usage();
        return ExitCode::SUCCESS;
    }

    if command == "tls-check" {
        let (Some(certificate), Some(key)) = (args.get(2), args.get(3)) else {
            usage();
            return ExitCode::from(2);
        };
        return match tls::load_server_config(Path::new(certificate), Path::new(key)) {
            Ok(_) => {
                println!("TLS material is valid");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("TLS material error: {error}");
                ExitCode::from(2)
            }
        };
    }

    if unsafe { libc_geteuid() } != 0 {
        eprintln!("multidesktopd must run as root so it can create isolated desktop sessions.");
        return ExitCode::from(1);
    }

    let config = match Config::load(Path::new(config_path)) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("configuration error: {error}");
            return ExitCode::from(2);
        }
    };

    if command == "check" {
        println!("configuration is valid");
        return ExitCode::SUCCESS;
    }
    if command == "doctor" {
        let report = SessionManager::new(config).doctor();
        for item in &report.items {
            println!(
                "{} {}: {}",
                if item.ok { "OK" } else { "FAIL" },
                item.name,
                item.detail
            );
        }
        return if report.ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(4)
        };
    }
    if command == "pair" {
        let Some(id) = args.get(2) else {
            usage();
            return ExitCode::from(2);
        };
        match SessionManager::new(config).create_pairing(id) {
            Ok(pairing) => {
                println!(
                    "desktop={}; code={}; expires_in_seconds={}",
                    pairing.desktop_id, pairing.code, pairing.expires_in_seconds
                );
                return ExitCode::SUCCESS;
            }
            Err(error) => {
                eprintln!("could not create pairing: {error}");
                return ExitCode::from(4);
            }
        }
    }
    if command != "serve" {
        usage();
        return ExitCode::from(2);
    }

    let tls_server = match (&config.tls_certificate, &config.tls_private_key) {
        (Some(certificate), Some(key)) => match tls::load_server_config(certificate, key) {
            Ok(server) => Some(server),
            Err(error) => {
                eprintln!("TLS configuration error: {error}");
                return ExitCode::from(2);
            }
        },
        (None, None) => None,
        _ => unreachable!("Config validates TLS paths as a pair"),
    };

    let listener = match TcpListener::bind(&config.listen) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("cannot bind {}: {error}", config.listen);
            return ExitCode::from(3);
        }
    };

    let manager = Arc::new(SessionManager::new(config.clone()));
    let limiter = Arc::new(AuthRateLimiter::new());
    eprintln!(
        "multi-desktop control plane listening on {} ({})",
        config.listen,
        if tls_server.is_some() {
            "TLS"
        } else {
            "loopback plaintext"
        }
    );

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let manager = Arc::clone(&manager);
                let limiter = Arc::clone(&limiter);
                let tls_server = tls_server.clone();
                thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(15)));
                    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(15)));
                    let peer = stream
                        .peer_addr()
                        .map(|address| address.ip().to_string())
                        .unwrap_or_else(|_| "unknown".to_owned());
                    let result = if let Some(server) = tls_server {
                        ServerConnection::new(server)
                            .map_err(std::io::Error::other)
                            .and_then(|connection| {
                                handle_client(
                                    StreamOwned::new(connection, stream),
                                    manager,
                                    peer,
                                    limiter,
                                )
                            })
                    } else {
                        handle_client(stream, manager, peer, limiter)
                    };
                    if let Err(error) = result {
                        eprintln!("client session ended: {error}");
                    }
                });
            }
            Err(error) => eprintln!("failed to accept connection: {error}"),
        }
    }
    ExitCode::SUCCESS
}

// Avoids an external crate in this base project. Linux only.
unsafe extern "C" {
    fn geteuid() -> u32;
}

unsafe fn libc_geteuid() -> u32 {
    unsafe { geteuid() }
}
