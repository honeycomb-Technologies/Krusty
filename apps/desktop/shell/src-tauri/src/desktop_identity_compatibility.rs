use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CANONICAL_DESKTOP_ID: &str = "io.mitsuro.desktop";
const LEGACY_DESKTOP_ID: &str = "io.krusty.desktop";
const EARLY_DEVELOPMENT_DESKTOP_ID: &str = "dev.krusty.desktop";
const MIGRATION_RECEIPT_FILE: &str = ".mitsuro-desktop-identity-v1.json";
const MIGRATION_LOCK_FILE: &str = ".io.mitsuro.desktop.identity-migration.lock";
const MAX_RECEIPT_BYTES: u64 = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum DesktopDataMigration {
    CanonicalAlreadyExists,
    NoLegacyData,
    Imported { from: PathBuf, to: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FileSnapshot {
    Directory,
    File { len: u64, sha256: [u8; 32] },
}

type TreeSnapshot = BTreeMap<PathBuf, FileSnapshot>;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationReceipt {
    version: u32,
    source_identifier: String,
    file_count: usize,
    total_bytes: u64,
    tree_fingerprint: String,
}

#[derive(Debug)]
struct MigrationLock {
    file: File,
}

impl MigrationLock {
    fn acquire(root: &Path) -> io::Result<Self> {
        let path = root.join(MIGRATION_LOCK_FILE);
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        configure_no_follow_lock_open(&mut options);
        let mut file = options.open(&path)?;
        validate_lock_handle(&path, &file)?;
        file.try_lock_exclusive().map_err(|error| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "another Mitsuro desktop identity migration owns {}: {error}",
                    path.display()
                ),
            )
        })?;
        validate_lock_handle(&path, &file)?;
        secure_private_file(&file)?;
        file.set_len(0)?;
        writeln!(
            file,
            "pid={}\ncreated_unix={}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        )?;
        file.sync_all()?;
        validate_lock_handle(&path, &file)?;
        Ok(Self { file })
    }
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(unix)]
fn configure_no_follow_lock_open(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
}

#[cfg(windows)]
fn configure_no_follow_lock_open(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn configure_no_follow_lock_open(_options: &mut OpenOptions) {}

fn validate_lock_handle(path: &Path, file: &File) -> io::Result<()> {
    let handle_metadata = file.metadata()?;
    if !handle_metadata.is_file() || handle_metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "desktop identity lock is not a regular file: {}",
                path.display()
            ),
        ));
    }

    let path_metadata = fs::symlink_metadata(path)?;
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "desktop identity lock path is not a regular file: {}",
                path.display()
            ),
        ));
    }
    validate_lock_identity(path, &handle_metadata, &path_metadata)
}

#[cfg(unix)]
fn validate_lock_identity(
    path: &Path,
    handle_metadata: &fs::Metadata,
    path_metadata: &fs::Metadata,
) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    if handle_metadata.nlink() != 1
        || handle_metadata.dev() != path_metadata.dev()
        || handle_metadata.ino() != path_metadata.ino()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "desktop identity lock was linked or replaced while opening: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_lock_identity(
    _path: &Path,
    _handle_metadata: &fs::Metadata,
    _path_metadata: &fs::Metadata,
) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_private_file(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn secure_private_file(_file: &File) -> io::Result<()> {
    Ok(())
}

pub fn migrate_legacy_desktop_data() -> io::Result<DesktopDataMigration> {
    let Some(data_root) = platform_data_root() else {
        return Ok(DesktopDataMigration::NoLegacyData);
    };
    migrate_legacy_desktop_data_at(&data_root)
}

/// Inject both names while cached frontend bundles and desktop shells overlap.
pub fn injected_connection_globals(port: u16) -> String {
    format!(
        "window.__MITSURO_SERVER_URL='http://localhost:{0}';window.__MITSURO_SERVER_TOKEN='local';window.__KRUSTY_SERVER_URL='http://localhost:{0}';window.__KRUSTY_SERVER_TOKEN='local';",
        port
    )
}

fn migrate_legacy_desktop_data_at(root: &Path) -> io::Result<DesktopDataMigration> {
    fs::create_dir_all(root)?;
    let _migration_lock = MigrationLock::acquire(root)?;
    let canonical = root.join(CANONICAL_DESKTOP_ID);
    let canonical_exists = checked_data_directory(&canonical)?;
    let legacy_candidates = legacy_data_directories(root)?;

    if canonical_exists {
        validate_existing_canonical(&canonical, &legacy_candidates)?;
        return Ok(DesktopDataMigration::CanonicalAlreadyExists);
    }
    if legacy_candidates.len() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "multiple prior desktop data namespaces coexist under {}; refusing to choose one and discard the other",
                root.display()
            ),
        ));
    }
    let Some(legacy) = legacy_candidates.into_iter().next() else {
        return Ok(DesktopDataMigration::NoLegacyData);
    };
    if !checked_data_directory(&legacy)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "legacy desktop data disappeared during migration: {}",
                legacy.display()
            ),
        ));
    }

    let source_snapshot = snapshot_tree(&legacy)?;
    let staging = unique_staging_path(root);
    fs::create_dir(&staging)?;
    secure_private_directory(&staging)?;
    if let Err(error) = copy_directory_contents(&legacy, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = validate_copied_snapshot(&legacy, &staging, &source_snapshot) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = write_migration_receipt(&staging, &legacy, &source_snapshot) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = sync_directory(&staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = validate_source_snapshot(&legacy, &source_snapshot) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    // Publish only after a stable source snapshot, exact destination proof, and
    // durable receipt. The process lock prevents a no-clobber race between two
    // cooperating Mitsuro desktop generations.
    if checked_data_directory(&canonical)? {
        let _ = fs::remove_dir_all(&staging);
        validate_existing_canonical(&canonical, &legacy_data_directories(root)?)?;
        return Ok(DesktopDataMigration::CanonicalAlreadyExists);
    }
    if let Err(error) = fs::rename(&staging, &canonical) {
        let _ = fs::remove_dir_all(&staging);
        if checked_data_directory(&canonical)? {
            validate_existing_canonical(&canonical, &legacy_data_directories(root)?)?;
            return Ok(DesktopDataMigration::CanonicalAlreadyExists);
        }
        return Err(error);
    }
    sync_directory(root).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "canonical desktop data was published at {} but syncing its parent directory failed: {error}; do not run the prior desktop app and restart Mitsuro to revalidate authority",
                canonical.display()
            ),
        )
    })?;
    Ok(DesktopDataMigration::Imported {
        from: legacy,
        to: canonical,
    })
}

fn legacy_data_directories(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut directories = Vec::new();
    for identifier in [LEGACY_DESKTOP_ID, EARLY_DEVELOPMENT_DESKTOP_ID] {
        let candidate = root.join(identifier);
        if checked_data_directory(&candidate)? {
            directories.push(candidate);
        }
    }
    Ok(directories)
}

fn checked_data_directory(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "refusing desktop identity migration through symlink {}",
                path.display()
            ),
        )),
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "desktop identity path is not a directory: {}",
                path.display()
            ),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn secure_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn secure_private_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn validate_existing_canonical(canonical: &Path, legacy: &[PathBuf]) -> io::Result<()> {
    if !checked_data_directory(canonical)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "canonical desktop data directory disappeared during validation: {}",
                canonical.display()
            ),
        ));
    }
    if legacy.is_empty() {
        let receipt = canonical.join(MIGRATION_RECEIPT_FILE);
        match fs::symlink_metadata(&receipt) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "desktop migration receipt {} requires its preserved prior source tree, but that tree is missing; restore it or complete an explicit compatibility-retirement cutover",
                        receipt.display()
                    ),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        return Ok(());
    }
    if legacy.len() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "canonical desktop data coexists with multiple prior namespaces; preserve every tree and resolve authority explicitly",
        ));
    }
    validate_migration_receipt(canonical, &legacy[0])
}

fn validate_migration_receipt(canonical: &Path, legacy: &Path) -> io::Result<()> {
    let path = canonical.join(MIGRATION_RECEIPT_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "canonical and prior desktop data coexist without migration receipt {}",
                    path.display()
                ),
            ));
        }
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_RECEIPT_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("desktop migration receipt is invalid: {}", path.display()),
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(&path)?
        .take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "desktop migration receipt exceeds its size limit",
        ));
    }
    let receipt: MigrationReceipt = serde_json::from_slice(&bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to parse desktop migration receipt: {error}"),
        )
    })?;
    let source_identifier = legacy.file_name().and_then(|name| name.to_str());
    if receipt.version != 1
        || source_identifier != Some(receipt.source_identifier.as_str())
        || !is_lower_hex_sha256(&receipt.tree_fingerprint)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "desktop migration receipt {} does not identify the exact prior data tree",
                path.display()
            ),
        ));
    }

    let snapshot = snapshot_tree(legacy)?;
    let actual_fingerprint = tree_fingerprint(&snapshot);
    if receipt.file_count != snapshot.len()
        || receipt.total_bytes != snapshot_total_bytes(&snapshot)
        || receipt.tree_fingerprint != digest_hex(&actual_fingerprint)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "prior desktop data at {} changed after Mitsuro imported it; both trees were preserved and authority must be resolved explicitly",
                legacy.display()
            ),
        ));
    }
    Ok(())
}

fn write_migration_receipt(
    destination: &Path,
    source: &Path,
    snapshot: &TreeSnapshot,
) -> io::Result<()> {
    let source_identifier = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "legacy desktop identifier is invalid",
            )
        })?;
    let receipt = MigrationReceipt {
        version: 1,
        source_identifier: source_identifier.to_string(),
        file_count: snapshot.len(),
        total_bytes: snapshot_total_bytes(snapshot),
        tree_fingerprint: digest_hex(&tree_fingerprint(snapshot)),
    };
    let bytes = serde_json::to_vec_pretty(&receipt).map_err(io::Error::other)?;
    let path = destination.join(MIGRATION_RECEIPT_FILE);
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()
}

fn validate_copied_snapshot(
    source: &Path,
    destination: &Path,
    expected: &TreeSnapshot,
) -> io::Result<()> {
    validate_source_snapshot(source, expected)?;
    let copied = snapshot_tree(destination)?;
    if copied != *expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "desktop web-data staging copy does not exactly match its stable source snapshot",
        ));
    }
    Ok(())
}

fn validate_source_snapshot(source: &Path, expected: &TreeSnapshot) -> io::Result<()> {
    if snapshot_tree(source)? != *expected {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "legacy desktop web data changed during migration; close the old desktop app and retry",
        ));
    }
    Ok(())
}

fn snapshot_tree(root: &Path) -> io::Result<TreeSnapshot> {
    let mut snapshot = BTreeMap::new();
    snapshot_directory(root, root, &mut snapshot)?;
    Ok(snapshot)
}

fn snapshot_directory(
    root: &Path,
    directory: &Path,
    snapshot: &mut TreeSnapshot,
) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "refusing to inspect desktop data symlink {}",
                    path.display()
                ),
            ));
        }
        if file_type.is_dir() {
            let relative = path
                .strip_prefix(root)
                .map_err(io::Error::other)?
                .to_path_buf();
            snapshot.insert(relative, FileSnapshot::Directory);
            snapshot_directory(root, &path, snapshot)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(io::Error::other)?
                .to_path_buf();
            let (len, sha256) = fingerprint_file(&path)?;
            snapshot.insert(relative, FileSnapshot::File { len, sha256 });
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "refusing to import special desktop data file {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn fingerprint_file(path: &Path) -> io::Result<(u64, [u8; 32])> {
    let mut file = File::open(path)?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut len = 0_u64;
    let mut hash = Sha256::new();
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        len = len
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("desktop data file size overflow"))?;
        hash.update(&buffer[..read]);
    }
    Ok((len, hash.finalize().into()))
}

fn snapshot_total_bytes(snapshot: &TreeSnapshot) -> u64 {
    snapshot
        .values()
        .filter_map(|entry| match entry {
            FileSnapshot::Directory => None,
            FileSnapshot::File { len, .. } => Some(*len),
        })
        .sum()
}

fn tree_fingerprint(snapshot: &TreeSnapshot) -> [u8; 32] {
    let mut hash = Sha256::new();
    for (path, file) in snapshot {
        let components: Vec<_> = path.components().collect();
        hash.update((components.len() as u64).to_le_bytes());
        for component in components {
            hash_os_str(&mut hash, component.as_os_str());
        }
        match file {
            FileSnapshot::Directory => hash.update(b"directory"),
            FileSnapshot::File { len, sha256 } => {
                hash.update(b"file");
                hash.update(len.to_le_bytes());
                hash.update(sha256);
            }
        }
    }
    hash.finalize().into()
}

fn hash_os_str(hash: &mut Sha256, value: &OsStr) {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bytes = value.as_bytes();
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let units: Vec<u16> = value.encode_wide().collect();
        hash.update((units.len() as u64).to_le_bytes());
        for unit in units {
            hash.update(unit.to_le_bytes());
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let text = value.to_string_lossy();
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
    }
}

fn digest_hex(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn copy_directory_contents(source: &Path, destination: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if file_type.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("refusing to import symlink {}", source_path.display()),
            ));
        }
        if file_type.is_dir() {
            fs::create_dir(&destination_path)?;
            copy_directory_contents(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path)?;
            File::open(&destination_path)?.sync_all()?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "refusing to import special desktop data file {}",
                    source_path.display()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn unique_staging_path(root: &Path) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    root.join(format!(
        ".{CANONICAL_DESKTOP_ID}.migration-{}-{nonce}",
        std::process::id()
    ))
}

#[cfg(target_os = "linux")]
fn platform_data_root() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".local/share")))
}

#[cfg(target_os = "macos")]
fn platform_data_root() -> Option<PathBuf> {
    // Wry does not expose WKWebView's data-directory authority on macOS. An
    // Application Support copy would therefore look successful without
    // proving localStorage/WebKit continuity. Keep this an explicit Apple-side
    // migration until it is verified on a signed macOS build.
    None
}

#[cfg(target_os = "windows")]
fn platform_data_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_data_root() -> Option<PathBuf> {
    None
}

#[cfg(target_os = "linux")]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mitsuro-desktop-migration-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }

    #[test]
    fn imports_only_when_canonical_is_absent_and_preserves_legacy() {
        let root = test_root("import");
        let legacy = root.join(LEGACY_DESKTOP_ID);
        fs::create_dir_all(legacy.join("localstorage")).unwrap();
        fs::write(legacy.join("localstorage/state.sqlite"), b"prior-state").unwrap();

        let result = migrate_legacy_desktop_data_at(&root).unwrap();
        assert!(matches!(result, DesktopDataMigration::Imported { .. }));
        assert_eq!(
            fs::read(
                root.join(CANONICAL_DESKTOP_ID)
                    .join("localstorage/state.sqlite")
            )
            .unwrap(),
            b"prior-state"
        );
        assert!(legacy.join("localstorage/state.sqlite").exists());
        assert!(root
            .join(CANONICAL_DESKTOP_ID)
            .join(MIGRATION_RECEIPT_FILE)
            .is_file());
        assert_eq!(
            migrate_legacy_desktop_data_at(&root).unwrap(),
            DesktopDataMigration::CanonicalAlreadyExists
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_directory_without_legacy_needs_no_migration_receipt() {
        let root = test_root("canonical-wins");
        let canonical = root.join(CANONICAL_DESKTOP_ID);
        fs::create_dir_all(&canonical).unwrap();
        fs::write(canonical.join("canonical-only"), b"canonical").unwrap();

        assert_eq!(
            migrate_legacy_desktop_data_at(&root).unwrap(),
            DesktopDataMigration::CanonicalAlreadyExists
        );
        assert!(canonical.join("canonical-only").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coexistence_without_receipt_fails_closed() {
        let root = test_root("ambiguous");
        let legacy = root.join(LEGACY_DESKTOP_ID);
        let canonical = root.join(CANONICAL_DESKTOP_ID);
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&canonical).unwrap();
        fs::write(legacy.join("legacy-only"), b"legacy").unwrap();
        fs::write(canonical.join("canonical-only"), b"canonical").unwrap();

        let error = migrate_legacy_desktop_data_at(&root).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(legacy.join("legacy-only").exists());
        assert!(canonical.join("canonical-only").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn multiple_prior_namespaces_fail_closed_without_selecting_one() {
        let root = test_root("source-priority");
        let production = root.join(LEGACY_DESKTOP_ID);
        let development = root.join(EARLY_DEVELOPMENT_DESKTOP_ID);
        fs::create_dir_all(&production).unwrap();
        fs::create_dir_all(&development).unwrap();
        fs::write(production.join("source"), b"production").unwrap();
        fs::write(development.join("source"), b"development").unwrap();

        let error = migrate_legacy_desktop_data_at(&root).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(production.join("source").exists());
        assert!(development.join("source").exists());
        assert!(!root.join(CANONICAL_DESKTOP_ID).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coexistence_rejects_a_receipt_for_the_wrong_source() {
        let root = test_root("wrong-source");
        let legacy = root.join(LEGACY_DESKTOP_ID);
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("state"), b"legacy").unwrap();
        migrate_legacy_desktop_data_at(&root).unwrap();

        let receipt_path = root.join(CANONICAL_DESKTOP_ID).join(MIGRATION_RECEIPT_FILE);
        let mut receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
        receipt["source_identifier"] =
            serde_json::Value::String(EARLY_DEVELOPMENT_DESKTOP_ID.to_string());
        fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();

        let error = migrate_legacy_desktop_data_at(&root).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(legacy.join("state").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migrated_canonical_rejects_a_missing_preserved_source() {
        let root = test_root("missing-source");
        let legacy = root.join(LEGACY_DESKTOP_ID);
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("state"), b"legacy").unwrap();
        migrate_legacy_desktop_data_at(&root).unwrap();
        fs::remove_dir_all(&legacy).unwrap();

        let error = migrate_legacy_desktop_data_at(&root).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("compatibility-retirement"));
        assert!(root.join(CANONICAL_DESKTOP_ID).is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coexistence_rejects_malformed_snapshot_digest() {
        let root = test_root("bad-digest");
        let legacy = root.join(LEGACY_DESKTOP_ID);
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("state"), b"legacy").unwrap();
        migrate_legacy_desktop_data_at(&root).unwrap();

        let receipt_path = root.join(CANONICAL_DESKTOP_ID).join(MIGRATION_RECEIPT_FILE);
        let mut receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
        receipt["tree_fingerprint"] = serde_json::Value::String("A".repeat(64));
        fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();

        let error = migrate_legacy_desktop_data_at(&root).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_added_and_removed_prior_files_fail_closed() {
        for mutation in ["change", "add", "remove"] {
            let root = test_root(mutation);
            let legacy = root.join(LEGACY_DESKTOP_ID);
            fs::create_dir_all(&legacy).unwrap();
            fs::write(legacy.join("state"), b"before").unwrap();
            fs::write(legacy.join("retained"), b"retained").unwrap();
            migrate_legacy_desktop_data_at(&root).unwrap();

            match mutation {
                "change" => fs::write(legacy.join("state"), b"after").unwrap(),
                "add" => fs::write(legacy.join("added"), b"new").unwrap(),
                "remove" => fs::remove_file(legacy.join("retained")).unwrap(),
                _ => unreachable!(),
            }

            let error = migrate_legacy_desktop_data_at(&root).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(root.join(CANONICAL_DESKTOP_ID).is_dir());
            assert!(legacy.is_dir());
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn advisory_lock_recovers_after_owner_exit_without_unlinking() {
        let root = test_root("advisory-lock");
        fs::create_dir_all(&root).unwrap();
        let first = MigrationLock::acquire(&root).unwrap();
        let error = MigrationLock::acquire(&root).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        drop(first);
        let second = MigrationLock::acquire(&root).unwrap();
        assert!(root.join(MIGRATION_LOCK_FILE).is_file());
        drop(second);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn advisory_lock_rejects_symlinks_without_truncating_their_target() {
        use std::os::unix::fs::symlink;

        let root = test_root("lock-symlink");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("must-not-change");
        fs::write(&target, b"preserved").unwrap();
        symlink(&target, root.join(MIGRATION_LOCK_FILE)).unwrap();

        assert!(MigrationLock::acquire(&root).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"preserved");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn advisory_lock_rejects_hardlinks_without_truncating_their_target() {
        let root = test_root("lock-hardlink");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("must-not-change");
        fs::write(&target, b"preserved").unwrap();
        fs::hard_link(&target, root.join(MIGRATION_LOCK_FILE)).unwrap();

        assert!(MigrationLock::acquire(&root).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"preserved");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changing_source_is_rejected_before_receipt_or_publish() {
        let root = test_root("source-change");
        let source = root.join(LEGACY_DESKTOP_ID);
        let staging = root.join("staging");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(source.join("state.sqlite"), b"before").unwrap();
        let expected = snapshot_tree(&source).unwrap();
        copy_directory_contents(&source, &staging).unwrap();
        fs::write(source.join("state.sqlite"), b"after").unwrap();

        let error = validate_copied_snapshot(&source, &staging, &expected).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(!staging.join(MIGRATION_RECEIPT_FILE).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn connection_globals_bridge_cached_and_canonical_frontends() {
        let script = injected_connection_globals(4321);
        assert!(script.contains("__MITSURO_SERVER_URL='http://localhost:4321'"));
        assert!(script.contains("__MITSURO_SERVER_TOKEN='local'"));
        assert!(script.contains("__KRUSTY_SERVER_URL='http://localhost:4321'"));
        assert!(script.contains("__KRUSTY_SERVER_TOKEN='local'"));
    }

    #[cfg(unix)]
    #[test]
    fn imported_canonical_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let root = test_root("private-root");
        let legacy = root.join(LEGACY_DESKTOP_ID);
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("state"), b"private").unwrap();

        migrate_legacy_desktop_data_at(&root).unwrap();

        let mode = fs::metadata(root.join(CANONICAL_DESKTOP_ID))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn identity_roots_cannot_be_symlinks() {
        use std::os::unix::fs::symlink;

        for identifier in [LEGACY_DESKTOP_ID, CANONICAL_DESKTOP_ID] {
            let root = test_root("symlink-root");
            let target = root.join("real-data");
            fs::create_dir_all(&target).unwrap();
            fs::write(target.join("state"), b"preserved").unwrap();
            symlink(&target, root.join(identifier)).unwrap();

            let error = migrate_legacy_desktop_data_at(&root).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(fs::read(target.join("state")).unwrap(), b"preserved");
            fs::remove_dir_all(root).unwrap();
        }
    }
}
