mod config;
mod pairing;
mod protocol;
mod session;

use std::{env, net::TcpListener, path::Path, process::ExitCode, sync::Arc, thread};

use config::Config;
use protocol::handle_client;
use session::SessionManager;

const DEFAULT_CONFIG: &str = "/etc/multi-desktop/multi-desktop.conf";

fn usage() {
    eprintln!(
        "Usage:\n  multidesktopd serve [config-path]\n  multidesktopd check [config-path]\n  multidesktopd doctor [config-path]\n  multidesktopd pair <desktop-id> [config-path]\n\nThe daemon must run as root. It provisions and supervises isolated desktop sessions."
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

    let listener = match TcpListener::bind(&config.listen) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("cannot bind {}: {error}", config.listen);
            return ExitCode::from(3);
        }
    };

    let manager = Arc::new(SessionManager::new(config.clone()));
    eprintln!("multi-desktop control plane listening on {}", config.listen);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let manager = Arc::clone(&manager);
                thread::spawn(move || {
                    if let Err(error) = handle_client(stream, manager) {
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
