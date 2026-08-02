//! Bounded read/copy bridge for pre-Mitsuro Game Boy Color data.
//!
//! The old root is never moved, deleted, or written. Existing canonical files
//! always win. Missing files are copied one at a time through a synced
//! same-directory temporary file and a no-clobber hard link, making the bridge
//! resumable without ever exposing a partial canonical file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const CANONICAL_RELATIVE_ROOT: &str = ".config/mitsuro/gameboy-color";
const LEGACY_RELATIVE_ROOT: &str = ".config/krusty/gameboy-color";

#[derive(Debug)]
pub(super) struct GameBoyColorDirs {
    canonical_root: PathBuf,
    legacy_root: Option<PathBuf>,
    pub(super) cores: PathBuf,
    pub(super) system: PathBuf,
    pub(super) saves: PathBuf,
    pub(super) states: PathBuf,
    pub(super) roms: PathBuf,
}

impl GameBoyColorDirs {
    pub(super) fn discover() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self::for_home(&home)
    }

    fn for_home(home: &Path) -> Self {
        let canonical_root = home.join(CANONICAL_RELATIVE_ROOT);
        let candidate_legacy_root = home.join(LEGACY_RELATIVE_ROOT);
        let legacy_root = match validate_optional_legacy_root(&candidate_legacy_root) {
            Ok(true) => {
                if let Err(error) =
                    copy_legacy_tree_non_overwriting(&candidate_legacy_root, &canonical_root)
                {
                    tracing::warn!(
                        legacy = %candidate_legacy_root.display(),
                        canonical = %canonical_root.display(),
                        %error,
                        "Game Boy Color data copy was not completed; retaining read-only legacy fallback"
                    );
                }
                Some(candidate_legacy_root)
            }
            Ok(false) => None,
            Err(error) => {
                tracing::warn!(
                    legacy = %candidate_legacy_root.display(),
                    %error,
                    "Refusing unsafe legacy Game Boy Color data tree"
                );
                None
            }
        };

        let result = Self {
            cores: canonical_root.join("cores"),
            system: canonical_root.join("system"),
            saves: canonical_root.join("saves"),
            states: canonical_root.join("states"),
            roms: canonical_root.join("roms"),
            canonical_root,
            legacy_root,
        };
        for directory in [
            &result.cores,
            &result.system,
            &result.saves,
            &result.states,
            &result.roms,
        ] {
            if let Err(error) = fs::create_dir_all(directory) {
                tracing::warn!(path = %directory.display(), %error, "Failed to create Game Boy Color directory");
            }
        }
        result
    }

    /// Resolve a read while keeping every new write on the canonical path.
    pub(super) fn read_path(&self, canonical_path: &Path) -> PathBuf {
        if canonical_path.is_file() {
            return canonical_path.to_path_buf();
        }
        self.legacy_path_for(canonical_path)
            .filter(|path| regular_file(path))
            .unwrap_or_else(|| canonical_path.to_path_buf())
    }

    /// Prefer canonical directory contents, falling back only when it is empty.
    pub(super) fn preferred_read_directory(&self, canonical: &Path) -> PathBuf {
        if directory_has_entries(canonical) {
            return canonical.to_path_buf();
        }
        self.legacy_path_for(canonical)
            .filter(|path| path.is_dir())
            .unwrap_or_else(|| canonical.to_path_buf())
    }

    /// Additional read-only search root. Callers keep the canonical root first.
    pub(super) fn legacy_read_directory(&self, canonical: &Path) -> Option<PathBuf> {
        self.legacy_path_for(canonical).filter(|path| path.is_dir())
    }

    fn legacy_path_for(&self, canonical_path: &Path) -> Option<PathBuf> {
        let relative = canonical_path.strip_prefix(&self.canonical_root).ok()?;
        Some(self.legacy_root.as_ref()?.join(relative))
    }
}

#[derive(Debug)]
struct PlannedCopy {
    source: PathBuf,
    destination: PathBuf,
    permissions: fs::Permissions,
}

fn validate_optional_legacy_root(root: &Path) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_dir() {
        return Err(unsafe_entry_error(root));
    }
    validate_tree(root)?;
    Ok(true)
}

fn validate_tree(root: &Path) -> io::Result<()> {
    for entry in sorted_entries(root)? {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            validate_tree(&path)?;
        } else if !file_type.is_file() {
            return Err(unsafe_entry_error(&path));
        }
    }
    Ok(())
}

fn copy_legacy_tree_non_overwriting(source_root: &Path, canonical_root: &Path) -> io::Result<()> {
    let mut directories = Vec::new();
    let mut files = Vec::new();
    plan_copy_tree(source_root, canonical_root, &mut directories, &mut files)?;

    directories.sort_by_key(|path| path.components().count());
    directories.dedup();
    for directory in directories {
        create_checked_directory(&directory)?;
    }
    for (index, planned) in files.into_iter().enumerate() {
        copy_file_no_clobber(&planned, index)?;
    }
    Ok(())
}

fn plan_copy_tree(
    source: &Path,
    destination: &Path,
    directories: &mut Vec<PathBuf>,
    files: &mut Vec<PlannedCopy>,
) -> io::Result<()> {
    let source_metadata = fs::symlink_metadata(source)?;
    if !source_metadata.file_type().is_dir() {
        return Err(unsafe_entry_error(source));
    }
    if let Ok(destination_metadata) = fs::symlink_metadata(destination) {
        if !destination_metadata.file_type().is_dir() {
            return Err(collision_error(destination));
        }
    }
    directories.push(destination.to_path_buf());

    for entry in sorted_entries(source)? {
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            plan_copy_tree(&source_path, &destination_path, directories, files)?;
        } else if file_type.is_file() {
            match fs::symlink_metadata(&destination_path) {
                Ok(metadata) if metadata.file_type().is_file() => {
                    if !files_equal(&source_path, &destination_path)? {
                        return Err(collision_error(&destination_path));
                    }
                }
                Ok(_) => return Err(collision_error(&destination_path)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    files.push(PlannedCopy {
                        source: source_path,
                        destination: destination_path,
                        permissions: entry.metadata()?.permissions(),
                    });
                }
                Err(error) => return Err(error),
            }
        } else {
            return Err(unsafe_entry_error(&source_path));
        }
    }
    Ok(())
}

fn create_checked_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(collision_error(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            sync_directory(path.parent().unwrap_or(path))
        }
        Err(error) => Err(error),
    }
}

fn copy_file_no_clobber(planned: &PlannedCopy, index: usize) -> io::Result<()> {
    let parent = planned
        .destination
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "copy target has no parent"))?;
    let file_name = planned
        .destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data");
    let temporary = parent.join(format!(
        ".{file_name}.mitsuro-import-{}-{index}",
        std::process::id()
    ));
    let result = (|| {
        let mut source = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&planned.source)?;
        if !source.metadata()?.is_file() {
            return Err(unsafe_entry_error(&planned.source));
        }
        let mut staged = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        io::copy(&mut source, &mut staged)?;
        staged.flush()?;
        staged.set_permissions(planned.permissions.clone())?;
        staged.sync_all()?;
        drop(staged);
        fs::hard_link(&temporary, &planned.destination).map_err(|error| {
            io::Error::new(
                if planned.destination.exists() {
                    io::ErrorKind::AlreadyExists
                } else {
                    error.kind()
                },
                format!(
                    "failed to publish legacy Game Boy Color file without overwriting {}: {error}",
                    planned.destination.display()
                ),
            )
        })?;
        sync_directory(parent)
    })();
    let cleanup = fs::remove_file(&temporary);
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(error)) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        (Ok(()), Err(error)) => Err(error),
        (Err(error), _) => Err(error),
    }
}

fn files_equal(left: &Path, right: &Path) -> io::Result<bool> {
    if fs::metadata(left)?.len() != fs::metadata(right)?.len() {
        return Ok(false);
    }
    let mut left = File::open(left)?;
    let mut right = File::open(right)?;
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];
    loop {
        let left_count = left.read(&mut left_buffer)?;
        let right_count = right.read(&mut right_buffer)?;
        if left_count != right_count || left_buffer[..left_count] != right_buffer[..right_count] {
            return Ok(false);
        }
        if left_count == 0 {
            return Ok(true);
        }
    }
}

fn sorted_entries(path: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_file())
        .unwrap_or(false)
}

fn directory_has_entries(path: &Path) -> bool {
    fs::read_dir(path)
        .ok()
        .and_then(|mut entries| entries.next())
        .is_some()
}

fn collision_error(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "canonical Game Boy Color data collision at {}; refusing to overwrite",
            path.display()
        ),
    )
}

fn unsafe_entry_error(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "legacy Game Boy Color entry is a symlink or special file: {}",
            path.display()
        ),
    )
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::symlink;

    #[test]
    fn copies_old_tree_without_mutating_source_and_keeps_canonical_writes() {
        let temp = tempfile::tempdir().expect("home");
        let old = temp.path().join(LEGACY_RELATIVE_ROOT);
        fs::create_dir_all(old.join("saves")).expect("old saves");
        fs::create_dir_all(old.join("states")).expect("old states");
        fs::write(old.join("saves/game.srm"), "battery").expect("old save");
        fs::write(old.join("states/game.state"), "state").expect("old state");

        let dirs = GameBoyColorDirs::for_home(temp.path());

        assert_eq!(
            fs::read_to_string(dirs.saves.join("game.srm")).unwrap(),
            "battery"
        );
        assert_eq!(
            fs::read_to_string(dirs.states.join("game.state")).unwrap(),
            "state"
        );
        assert_eq!(
            fs::read_to_string(old.join("saves/game.srm")).unwrap(),
            "battery"
        );
        assert_eq!(
            dirs.read_path(&dirs.saves.join("game.srm")),
            dirs.saves.join("game.srm")
        );
    }

    #[test]
    fn collision_refuses_all_copying_and_read_bridge_prefers_canonical_file() {
        let temp = tempfile::tempdir().expect("home");
        let old = temp.path().join(LEGACY_RELATIVE_ROOT);
        let canonical = temp.path().join(CANONICAL_RELATIVE_ROOT);
        fs::create_dir_all(old.join("saves")).expect("old saves");
        fs::create_dir_all(old.join("states")).expect("old states");
        fs::create_dir_all(canonical.join("saves")).expect("canonical saves");
        fs::write(old.join("saves/game.srm"), "old").expect("old save");
        fs::write(old.join("states/game.state"), "old state").expect("old state");
        fs::write(canonical.join("saves/game.srm"), "canonical").expect("canonical save");

        let error = copy_legacy_tree_non_overwriting(&old, &canonical)
            .expect_err("collision must fail before copying");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(!canonical.join("states/game.state").exists());

        let dirs = GameBoyColorDirs::for_home(temp.path());
        assert_eq!(
            dirs.read_path(&dirs.saves.join("game.srm")),
            dirs.saves.join("game.srm")
        );
        assert_eq!(
            dirs.read_path(&dirs.states.join("game.state")),
            old.join("states/game.state")
        );
        assert_eq!(
            fs::read_to_string(old.join("saves/game.srm")).unwrap(),
            "old"
        );
    }

    #[test]
    fn refuses_symlinks_before_publishing_any_file() {
        let temp = tempfile::tempdir().expect("home");
        let old = temp.path().join(LEGACY_RELATIVE_ROOT);
        let canonical = temp.path().join(CANONICAL_RELATIVE_ROOT);
        fs::create_dir_all(old.join("roms")).expect("old roms");
        fs::write(old.join("roms/safe.gbc"), "rom").expect("safe rom");
        symlink("safe.gbc", old.join("roms/link.gbc")).expect("old symlink");

        let error = copy_legacy_tree_non_overwriting(&old, &canonical)
            .expect_err("symlink must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!canonical.exists());
    }

    #[test]
    fn refuses_special_files_before_publishing_any_file() {
        let temp = tempfile::tempdir().expect("home");
        let old = temp.path().join(LEGACY_RELATIVE_ROOT);
        let canonical = temp.path().join(CANONICAL_RELATIVE_ROOT);
        fs::create_dir_all(old.join("saves")).expect("old saves");
        fs::write(old.join("saves/safe.srm"), "save").expect("safe save");
        let fifo = old.join("saves/unsafe.fifo");
        let fifo_path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).expect("fifo path");
        assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);

        let error = copy_legacy_tree_non_overwriting(&old, &canonical)
            .expect_err("special file must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!canonical.exists());
    }
}
