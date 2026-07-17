use std::collections::HashMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;

use crate::types::{Hello, HelloAck, ProtocolVersion};
use crate::{AuthError, IPC_KEY_BYTES, NONCE_BYTES};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct IpcKey([u8; IPC_KEY_BYTES]);

impl fmt::Debug for IpcKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("IpcKey([REDACTED])")
    }
}

impl IpcKey {
    pub fn from_bytes(bytes: [u8; IPC_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn generate() -> Self {
        let mut bytes = [0_u8; IPC_KEY_BYTES];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn load(path: &Path) -> Result<Self, AuthError> {
        validate_key_metadata(path)?;
        let mut file = fs::File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if bytes.len() != IPC_KEY_BYTES {
            return Err(AuthError::InvalidKeyLength {
                expected: IPC_KEY_BYTES,
                actual: bytes.len(),
            });
        }
        let mut key = [0_u8; IPC_KEY_BYTES];
        key.copy_from_slice(&bytes);
        Ok(Self(key))
    }

    /// Atomically create a new private key, or securely load the existing one.
    pub fn load_or_create(path: &Path) -> Result<Self, AuthError> {
        let parent = path.parent().ok_or_else(|| {
            AuthError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "IPC key path has no parent",
            ))
        })?;
        ensure_private_dir(parent)?;

        let key = Self::generate();
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        match options.open(path) {
            Ok(mut file) => {
                file.write_all(&key.0)?;
                file.sync_all()?;
                validate_key_metadata(path)?;
                Ok(key)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Self::load(path),
            Err(error) => Err(AuthError::Io(error)),
        }
    }

    pub fn hello(&self, client_id: impl Into<String>) -> Hello {
        let client_id = client_id.into();
        let issued_at_unix_ms = unix_time_millis();
        let nonce = random_hex_nonce();
        let mac = self.sign(&hello_signing_input(
            ProtocolVersion::CURRENT,
            &client_id,
            &nonce,
            issued_at_unix_ms,
        ));
        Hello {
            version: ProtocolVersion::CURRENT,
            client_id,
            nonce,
            issued_at_unix_ms,
            mac,
        }
    }

    pub fn verify_hello(
        &self,
        hello: &Hello,
        policy: AuthPolicy,
        now_unix_ms: i64,
    ) -> Result<ProtocolVersion, AuthError> {
        if hello.client_id.is_empty() || hello.client_id.len() > 256 {
            return Err(AuthError::InvalidClientId);
        }
        validate_nonce(&hello.nonce)?;
        let skew = hello.issued_at_unix_ms.abs_diff(now_unix_ms);
        if skew > policy.max_clock_skew.as_millis() as u64 {
            return Err(AuthError::StaleHello);
        }
        self.verify(
            &hello_signing_input(
                hello.version,
                &hello.client_id,
                &hello.nonce,
                hello.issued_at_unix_ms,
            ),
            &hello.mac,
        )?;
        hello.version.negotiate().map_err(|_| AuthError::InvalidMac)
    }

    pub fn hello_ack(
        &self,
        negotiated_version: ProtocolVersion,
        instance_id: impl Into<String>,
        daemon_version: impl Into<String>,
        client_nonce: impl Into<String>,
    ) -> HelloAck {
        let instance_id = instance_id.into();
        let daemon_version = daemon_version.into();
        let client_nonce = client_nonce.into();
        let server_nonce = random_hex_nonce();
        let server_time_unix_ms = unix_time_millis();
        let mac = self.sign(&ack_signing_input(
            negotiated_version,
            &instance_id,
            &daemon_version,
            &client_nonce,
            &server_nonce,
            server_time_unix_ms,
        ));
        HelloAck {
            version: negotiated_version,
            instance_id,
            daemon_version,
            client_nonce,
            server_nonce,
            server_time_unix_ms,
            mac,
        }
    }

    pub fn verify_hello_ack(
        &self,
        ack: &HelloAck,
        expected_client_nonce: &str,
        policy: AuthPolicy,
        now_unix_ms: i64,
    ) -> Result<(), AuthError> {
        if ack.client_nonce != expected_client_nonce {
            return Err(AuthError::InvalidNonce);
        }
        validate_nonce(&ack.server_nonce)?;
        let skew = ack.server_time_unix_ms.abs_diff(now_unix_ms);
        if skew > policy.max_clock_skew.as_millis() as u64 {
            return Err(AuthError::StaleHello);
        }
        self.verify(
            &ack_signing_input(
                ack.version,
                &ack.instance_id,
                &ack.daemon_version,
                &ack.client_nonce,
                &ack.server_nonce,
                ack.server_time_unix_ms,
            ),
            &ack.mac,
        )
    }

    fn sign(&self, input: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.0).expect("HMAC accepts a 32-byte key");
        mac.update(input);
        encode_hex(&mac.finalize().into_bytes())
    }

    fn verify(&self, input: &[u8], encoded_mac: &str) -> Result<(), AuthError> {
        let decoded = decode_hex(encoded_mac).ok_or(AuthError::InvalidMac)?;
        let mut mac = HmacSha256::new_from_slice(&self.0).expect("HMAC accepts a 32-byte key");
        mac.update(input);
        mac.verify_slice(&decoded)
            .map_err(|_| AuthError::InvalidMac)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AuthPolicy {
    pub max_clock_skew: Duration,
    pub replay_window: Duration,
}

impl Default for AuthPolicy {
    fn default() -> Self {
        Self {
            max_clock_skew: Duration::from_secs(60),
            replay_window: Duration::from_secs(120),
        }
    }
}

#[derive(Debug, Default)]
pub struct NonceReplayGuard {
    seen: Mutex<HashMap<String, i64>>,
}

impl NonceReplayGuard {
    pub fn check_and_record(
        &self,
        nonce: &str,
        policy: AuthPolicy,
        now_unix_ms: i64,
    ) -> Result<(), AuthError> {
        validate_nonce(nonce)?;
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cutoff = now_unix_ms
            .saturating_sub(i64::try_from(policy.replay_window.as_millis()).unwrap_or(i64::MAX));
        seen.retain(|_, observed_at| *observed_at >= cutoff);
        if seen.contains_key(nonce) {
            return Err(AuthError::ReplayedNonce);
        }
        seen.insert(nonce.to_string(), now_unix_ms);
        Ok(())
    }
}

pub fn unix_time_millis() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

/// Create or tighten the private directory used for Mako socket/key material.
pub fn ensure_private_dir(path: &Path) -> Result<(), AuthError> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() {
        return Err(AuthError::NotDirectory);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let expected_uid = crate::current_effective_uid();
        if metadata.uid() != expected_uid {
            return Err(AuthError::WrongDirectoryOwner {
                expected: expected_uid,
                actual: metadata.uid(),
            });
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }

    Ok(())
}

fn validate_key_metadata(path: &Path) -> Result<(), AuthError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(AuthError::KeyNotRegularFile);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let expected_uid = crate::current_effective_uid();
        if metadata.uid() != expected_uid {
            return Err(AuthError::WrongKeyOwner {
                expected: expected_uid,
                actual: metadata.uid(),
            });
        }
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(AuthError::InsecureKeyPermissions { mode });
        }
    }

    Ok(())
}

fn hello_signing_input(
    version: ProtocolVersion,
    client_id: &str,
    nonce: &str,
    issued_at_unix_ms: i64,
) -> Vec<u8> {
    format!(
        "mako-ipc-hello-v1\n{}.{}\n{}\n{}\n{}",
        version.major, version.minor, client_id, nonce, issued_at_unix_ms
    )
    .into_bytes()
}

fn ack_signing_input(
    version: ProtocolVersion,
    instance_id: &str,
    daemon_version: &str,
    client_nonce: &str,
    server_nonce: &str,
    server_time_unix_ms: i64,
) -> Vec<u8> {
    format!(
        "mako-ipc-ack-v1\n{}.{}\n{}\n{}\n{}\n{}\n{}",
        version.major,
        version.minor,
        instance_id,
        daemon_version,
        client_nonce,
        server_nonce,
        server_time_unix_ms
    )
    .into_bytes()
}

fn random_hex_nonce() -> String {
    let mut bytes = [0_u8; NONCE_BYTES];
    OsRng.fill_bytes(&mut bytes);
    encode_hex(&bytes)
}

fn validate_nonce(value: &str) -> Result<(), AuthError> {
    let decoded = decode_hex(value).ok_or(AuthError::InvalidNonce)?;
    if decoded.len() != NONCE_BYTES {
        return Err(AuthError::InvalidNonce);
    }
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    let mut decoded = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    for pair in bytes.chunks_exact(2) {
        let high = hex_digit(pair[0])?;
        let low = hex_digit(pair[1])?;
        decoded.push((high << 4) | low);
    }
    Some(decoded)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticates_hello_and_ack() {
        let key = IpcKey::generate();
        let hello = key.hello("test-client");
        let negotiated = key
            .verify_hello(&hello, AuthPolicy::default(), unix_time_millis())
            .unwrap();
        let ack = key.hello_ack(negotiated, "instance", "1.0", hello.nonce.clone());
        key.verify_hello_ack(
            &ack,
            &hello.nonce,
            AuthPolicy::default(),
            unix_time_millis(),
        )
        .unwrap();
    }

    #[test]
    fn rejects_tampered_hello() {
        let key = IpcKey::generate();
        let mut hello = key.hello("test-client");
        hello.client_id = "attacker".to_string();
        assert!(matches!(
            key.verify_hello(&hello, AuthPolicy::default(), unix_time_millis()),
            Err(AuthError::InvalidMac)
        ));
    }

    #[test]
    fn rejects_stale_hello() {
        let key = IpcKey::generate();
        let hello = key.hello("test-client");
        let later = hello.issued_at_unix_ms + 120_000;
        assert!(matches!(
            key.verify_hello(&hello, AuthPolicy::default(), later),
            Err(AuthError::StaleHello)
        ));
    }

    #[test]
    fn nonce_guard_rejects_replay() {
        let guard = NonceReplayGuard::default();
        let key = IpcKey::generate();
        let hello = key.hello("test-client");
        let now = unix_time_millis();
        guard
            .check_and_record(&hello.nonce, AuthPolicy::default(), now)
            .unwrap();
        assert!(matches!(
            guard.check_and_record(&hello.nonce, AuthPolicy::default(), now),
            Err(AuthError::ReplayedNonce)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn creates_private_key_and_directory() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("private");
        let key_path = directory.join("mako.ipc.key");
        let _key = IpcKey::load_or_create(&key_path).unwrap();

        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&key_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        IpcKey::load(&key_path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_insecure_existing_key() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let key_path = temp.path().join("key");
        fs::write(&key_path, [7_u8; IPC_KEY_BYTES]).unwrap();
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();

        assert!(matches!(
            IpcKey::load(&key_path),
            Err(AuthError::InsecureKeyPermissions { .. })
        ));
    }
}
