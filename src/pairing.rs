use crate::config::valid_identifier;
use std::{
    fs,
    fs::OpenOptions,
    io::{self, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const PAIRING_LIFETIME_SECONDS: u64 = 300;
const PAIRING_CODE_BYTES: usize = 16;
const DEVICE_TOKEN_BYTES: usize = 32;

#[derive(Debug)]
pub struct Pairing {
    pub desktop_id: String,
    pub code: String,
    pub expires_in_seconds: u64,
}

#[derive(Debug)]
pub struct DeviceCredentials {
    pub desktop_id: String,
    pub token: String,
}

pub struct PairingStore {
    root: PathBuf,
}

impl PairingStore {
    pub fn host_default() -> Self {
        Self::new(PathBuf::from("/var/lib/multi-desktop"))
    }

    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn create(&self, desktop_id: &str) -> io::Result<Pairing> {
        validate_desktop_id(desktop_id)?;
        self.prepare_directories()?;
        let code = random_hex(PAIRING_CODE_BYTES)?;
        let expires = now_seconds()? + PAIRING_LIFETIME_SECONDS;
        let path = self.pairings_dir().join(&code);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        writeln!(file, "desktop_id={desktop_id}")?;
        writeln!(file, "expires={expires}")?;
        Ok(Pairing {
            desktop_id: desktop_id.to_owned(),
            code,
            expires_in_seconds: PAIRING_LIFETIME_SECONDS,
        })
    }

    pub fn redeem(&self, code: &str) -> io::Result<DeviceCredentials> {
        if !is_hex(code, PAIRING_CODE_BYTES * 2) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid pairing code",
            ));
        }
        self.prepare_directories()?;
        let path = self.pairings_dir().join(code);
        let contents = fs::read_to_string(&path).map_err(|_| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "invalid or used pairing code",
            )
        })?;
        fs::rename(&path, self.consumed_dir().join(code)).map_err(|_| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "invalid or used pairing code",
            )
        })?;
        let (desktop_id, expires) = parse_pairing(&contents)?;
        if now_seconds()? > expires {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "pairing code expired",
            ));
        }
        let token = random_hex(DEVICE_TOKEN_BYTES)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.devices_dir().join(&token))?;
        writeln!(file, "{desktop_id}")?;
        Ok(DeviceCredentials { desktop_id, token })
    }

    pub fn desktop_for_token(&self, token: &str) -> io::Result<Option<String>> {
        if !is_hex(token, DEVICE_TOKEN_BYTES * 2) {
            return Ok(None);
        }
        let path = self.devices_dir().join(token);
        let Ok(id) = fs::read_to_string(path) else {
            return Ok(None);
        };
        let id = id.trim().to_owned();
        if valid_identifier(&id) {
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    fn prepare_directories(&self) -> io::Result<()> {
        for path in [self.pairings_dir(), self.consumed_dir(), self.devices_dir()] {
            fs::create_dir_all(&path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    fn pairings_dir(&self) -> PathBuf {
        self.root.join("pairings")
    }
    fn consumed_dir(&self) -> PathBuf {
        self.root.join("pairings-used")
    }
    fn devices_dir(&self) -> PathBuf {
        self.root.join("devices")
    }
}

fn parse_pairing(contents: &str) -> io::Result<(String, u64)> {
    let mut desktop_id = None;
    let mut expires = None;
    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("desktop_id=") {
            desktop_id = Some(value.to_owned());
        }
        if let Some(value) = line.strip_prefix("expires=") {
            expires = value.parse().ok();
        }
    }
    let id = desktop_id
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid pairing record"))?;
    validate_desktop_id(&id)?;
    Ok((
        id,
        expires
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid pairing record"))?,
    ))
}

fn validate_desktop_id(id: &str) -> io::Result<()> {
    if valid_identifier(id) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid desktop id",
        ))
    }
}

fn now_seconds() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .map_err(io::Error::other)
}

fn random_hex(bytes: usize) -> io::Result<String> {
    let mut raw = vec![0_u8; bytes];
    fs::File::open("/dev/urandom")?.read_exact(&mut raw)?;
    Ok(raw.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn is_hex(value: &str, expected_length: usize) -> bool {
    value.len() == expected_length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_is_single_use_and_creates_a_device_token() {
        let root =
            std::env::temp_dir().join(format!("multi-desktop-pairing-{}", now_seconds().unwrap()));
        let store = PairingStore::new(root.clone());
        let pairing = store.create("laptop").unwrap();
        let credentials = store.redeem(&pairing.code).unwrap();
        assert_eq!(credentials.desktop_id, "laptop");
        assert_eq!(
            store
                .desktop_for_token(&credentials.token)
                .unwrap()
                .as_deref(),
            Some("laptop")
        );
        assert!(store.redeem(&pairing.code).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
