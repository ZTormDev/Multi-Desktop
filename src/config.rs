use std::{
    collections::HashMap,
    fs, io,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: String,
    pub token: String,
    pub user_prefix: String,
    pub desktop_command: String,
    pub tls_certificate: Option<PathBuf>,
    pub tls_private_key: Option<PathBuf>,
}

impl Config {
    pub fn load(path: &Path) -> io::Result<Self> {
        let contents = fs::read_to_string(path)?;
        let mut values = HashMap::new();
        for raw in contents.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid line: {line}"),
                ));
            };
            values.insert(key.trim(), value.trim());
        }
        for key in values.keys() {
            if !matches!(
                *key,
                "listen"
                    | "token"
                    | "user_prefix"
                    | "desktop_command"
                    | "tls_certificate"
                    | "tls_private_key"
            ) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unknown configuration key: {key}"),
                ));
            }
        }
        let required = |name: &str| -> io::Result<String> {
            values
                .get(name)
                .map(|value| (*value).to_owned())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("missing {name}"))
                })
        };
        let token = required("token")?;
        if token.len() < 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "token must be at least 32 characters",
            ));
        }
        let user_prefix = values
            .get("user_prefix")
            .copied()
            .unwrap_or("mdesk-")
            .to_owned();
        if !valid_identifier(user_prefix.trim_end_matches('-')) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "user_prefix must be ASCII letters, digits or hyphens",
            ));
        }
        let listen = required("listen")?;
        let listen_address = listen.parse::<SocketAddr>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "listen must be an IP address and port, for example 192.168.1.10:47990",
            )
        })?;
        let desktop_command = values
            .get("desktop_command")
            .copied()
            .unwrap_or(
                "gamescope --backend headless --expose-wayland -W 1920 -H 1080 -r 60 -- /usr/local/bin/multi-desktop-session-inner startxfce4",
            )
            .trim()
            .to_owned();
        if desktop_command.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "desktop_command cannot be empty",
            ));
        }
        let optional_path = |name: &str| {
            values
                .get(name)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let tls_certificate = optional_path("tls_certificate");
        let tls_private_key = optional_path("tls_private_key");
        if tls_certificate.is_some() != tls_private_key.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "tls_certificate and tls_private_key must be configured together",
            ));
        }
        if !is_loopback(listen_address.ip()) && tls_certificate.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a non-loopback listen address requires tls_certificate and tls_private_key",
            ));
        }
        Ok(Self {
            listen,
            token,
            user_prefix,
            desktop_command,
            tls_certificate,
            tls_private_key,
        })
    }
}

fn is_loopback(address: IpAddr) -> bool {
    address.is_loopback()
}

pub fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 24
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_config(contents: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("multi-desktop-config-{unique}"));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn loads_valid_configuration() {
        let path =
            temporary_config("listen=127.0.0.1:47990\ntoken=01234567890123456789012345678901\n");
        let config = Config::load(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(config.user_prefix, "mdesk-");
        assert!(config.tls_certificate.is_none());
    }

    #[test]
    fn rejects_unknown_keys() {
        let path = temporary_config(
            "listen=not-an-address\ntoken=01234567890123456789012345678901\nextra=value\n",
        );
        let error = Config::load(&path).unwrap_err();
        fs::remove_file(path).unwrap();
        assert!(error.to_string().contains("unknown configuration key"));
    }

    #[test]
    fn rejects_lan_listen_without_tls() {
        let path =
            temporary_config("listen=192.168.1.10:47990\ntoken=01234567890123456789012345678901\n");
        let error = Config::load(&path).unwrap_err();
        fs::remove_file(path).unwrap();
        assert!(error.to_string().contains("requires tls_certificate"));
    }

    #[test]
    fn requires_both_tls_paths() {
        let path = temporary_config(
            "listen=127.0.0.1:47990\ntoken=01234567890123456789012345678901\ntls_certificate=/tmp/server.pem\n",
        );
        let error = Config::load(&path).unwrap_err();
        fs::remove_file(path).unwrap();
        assert!(error.to_string().contains("configured together"));
    }
}
