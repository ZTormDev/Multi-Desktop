use crate::config::valid_identifier;
use sha2::{Digest, Sha256};
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
const DEVICE_ID_HEX_LENGTH: usize = 16;

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

#[derive(Debug, Eq, PartialEq)]
pub struct PairedDevice {
    pub id: String,
    pub desktop_id: String,
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
        let digest = token_digest(&token);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.devices_dir().join(token_record_name(&digest)))?;
        writeln!(file, "{desktop_id}")?;
        Ok(DeviceCredentials { desktop_id, token })
    }

    pub fn desktop_for_token(&self, token: &str) -> io::Result<Option<String>> {
        if !is_hex(token, DEVICE_TOKEN_BYTES * 2) {
            return Ok(None);
        }
        self.prepare_directories()?;
        let digest = token_digest(token);
        let digest_path = self.devices_dir().join(token_record_name(&digest));
        let id = match fs::read_to_string(&digest_path) {
            Ok(id) => id,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Versions before 0.1 stored the bearer token itself as the
                // filename. Migrate it atomically on the first valid use.
                let legacy_path = self.devices_dir().join(token);
                let Ok(id) = fs::read_to_string(&legacy_path) else {
                    return Ok(None);
                };
                match fs::rename(&legacy_path, &digest_path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        fs::remove_file(&legacy_path)?;
                    }
                    Err(error) => return Err(error),
                }
                id
            }
            Err(error) => return Err(error),
        };
        let id = id.trim().to_owned();
        if valid_identifier(&id) {
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    /// Lists stable, non-secret device identifiers. Legacy plaintext records
    /// stay hidden until a successful authentication migrates them.
    pub fn list_desktop_devices(&self, desktop_id: &str) -> io::Result<Vec<PairedDevice>> {
        validate_desktop_id(desktop_id)?;
        self.prepare_directories()?;
        let mut devices = Vec::new();
        for entry in fs::read_dir(self.devices_dir())? {
            let entry = entry?;
            let record_name = entry.file_name().to_string_lossy().into_owned();
            let Some(digest) = record_name.strip_prefix("sha256-") else {
                continue;
            };
            if !is_lower_hex(digest, DEVICE_TOKEN_BYTES * 2)
                || fs::read_to_string(entry.path())?.trim() != desktop_id
            {
                continue;
            }
            devices.push(PairedDevice {
                id: digest[..DEVICE_ID_HEX_LENGTH].to_owned(),
                desktop_id: desktop_id.to_owned(),
            });
        }
        devices.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(devices)
    }

    pub fn revoke_device(&self, desktop_id: &str, device_id: &str) -> io::Result<bool> {
        validate_desktop_id(desktop_id)?;
        if !is_lower_hex(device_id, DEVICE_ID_HEX_LENGTH) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid device id",
            ));
        }
        let matches: Vec<_> = self
            .list_desktop_devices(desktop_id)?
            .into_iter()
            .filter(|device| device.id == device_id)
            .collect();
        if matches.is_empty() {
            return Ok(false);
        }
        if matches.len() != 1 {
            return Err(io::Error::other("ambiguous device id"));
        }
        for entry in fs::read_dir(self.devices_dir())? {
            let entry = entry?;
            let record_name = entry.file_name().to_string_lossy().into_owned();
            let Some(digest) = record_name.strip_prefix("sha256-") else {
                continue;
            };
            if digest.starts_with(device_id)
                && is_lower_hex(digest, DEVICE_TOKEN_BYTES * 2)
                && fs::read_to_string(entry.path())?.trim() == desktop_id
            {
                fs::remove_file(entry.path())?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Invalidates every paired-device credential for one desktop without
    /// touching the desktop's user account, HOME or running session.
    pub fn revoke_desktop_tokens(&self, desktop_id: &str) -> io::Result<usize> {
        validate_desktop_id(desktop_id)?;
        self.prepare_directories()?;
        let mut revoked = 0;
        for entry in fs::read_dir(self.devices_dir())? {
            let entry = entry?;
            let record_name = entry.file_name().to_string_lossy().into_owned();
            if !is_device_record_name(&record_name) {
                continue;
            }
            if fs::read_to_string(entry.path())?.trim() == desktop_id {
                fs::remove_file(entry.path())?;
                revoked += 1;
            }
        }
        Ok(revoked)
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

fn token_digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn token_record_name(digest: &str) -> String {
    format!("sha256-{digest}")
}

fn is_device_record_name(value: &str) -> bool {
    is_hex(value, DEVICE_TOKEN_BYTES * 2)
        || value
            .strip_prefix("sha256-")
            .is_some_and(|digest| is_lower_hex(digest, DEVICE_TOKEN_BYTES * 2))
}

fn is_hex(value: &str, expected_length: usize) -> bool {
    value.len() == expected_length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_lower_hex(value: &str, expected_length: usize) -> bool {
    value.len() == expected_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
        assert!(!store.devices_dir().join(&credentials.token).exists());
        assert!(
            store
                .devices_dir()
                .join(token_record_name(&token_digest(&credentials.token)))
                .exists()
        );
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

    #[test]
    fn revoking_a_desktop_leaves_other_desktop_tokens_valid() {
        let root =
            std::env::temp_dir().join(format!("multi-desktop-revoke-{}", now_seconds().unwrap()));
        let store = PairingStore::new(root.clone());
        let first = store.create("first").unwrap();
        let first = store.redeem(&first.code).unwrap();
        let second = store.create("second").unwrap();
        let second = store.redeem(&second.code).unwrap();
        assert_eq!(store.revoke_desktop_tokens("first").unwrap(), 1);
        assert_eq!(store.desktop_for_token(&first.token).unwrap(), None);
        assert_eq!(
            store.desktop_for_token(&second.token).unwrap().as_deref(),
            Some("second")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lists_and_revokes_one_device_without_exposing_its_token() {
        let root = std::env::temp_dir().join(format!(
            "multi-desktop-target-revoke-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = PairingStore::new(root.clone());
        let first = store.redeem(&store.create("desk").unwrap().code).unwrap();
        let second = store.redeem(&store.create("desk").unwrap().code).unwrap();
        let devices = store.list_desktop_devices("desk").unwrap();
        assert_eq!(devices.len(), 2);
        let expected_ids = [first.token.as_str(), second.token.as_str()]
            .map(token_digest)
            .map(|digest| digest[..DEVICE_ID_HEX_LENGTH].to_owned());
        assert!(
            devices
                .iter()
                .all(|device| expected_ids.contains(&device.id))
        );
        assert!(store.revoke_device("desk", &devices[0].id).unwrap());
        let valid = [first.token, second.token]
            .iter()
            .filter(|token| store.desktop_for_token(token).unwrap().is_some())
            .count();
        assert_eq!(valid, 1);
        assert!(!store.revoke_device("desk", &devices[0].id).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migrates_a_legacy_plaintext_token_filename_on_use() {
        let root = std::env::temp_dir().join(format!(
            "multi-desktop-token-migration-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = PairingStore::new(root.clone());
        store.prepare_directories().unwrap();
        let token = "a".repeat(DEVICE_TOKEN_BYTES * 2);
        fs::write(store.devices_dir().join(&token), "desk\n").unwrap();
        assert_eq!(
            store.desktop_for_token(&token).unwrap().as_deref(),
            Some("desk")
        );
        assert!(!store.devices_dir().join(&token).exists());
        assert!(
            store
                .devices_dir()
                .join(token_record_name(&token_digest(&token)))
                .exists()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
