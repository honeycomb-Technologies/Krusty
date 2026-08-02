use std::fs;
use std::path::{Path, PathBuf};

use super::model::DEFAULT_CREW_SLUGS;
use super::{
    HiveBootstrapResult, HiveCrewDocumentKind, HiveHomeDocument, HiveHomeDocumentKind,
    HiveHomeProfile,
};

pub fn bootstrap_hive_home(hive_home: &Path) -> std::io::Result<HiveBootstrapResult> {
    fs::create_dir_all(hive_home)?;

    let mut created_files = Vec::new();
    for kind in [
        HiveHomeDocumentKind::Soul,
        HiveHomeDocumentKind::Identity,
        HiveHomeDocumentKind::User,
        HiveHomeDocumentKind::Heartbeat,
        HiveHomeDocumentKind::Memory,
        HiveHomeDocumentKind::Channels,
    ] {
        if create_document_if_missing(
            hive_home,
            kind.preferred_file_name(),
            kind.default_content(),
        )? {
            created_files.push(kind.preferred_file_name().to_string());
        }
    }

    for slug in DEFAULT_CREW_SLUGS {
        let crew_dir = hive_home.join("crew").join(slug);
        fs::create_dir_all(&crew_dir)?;
        for kind in [
            HiveCrewDocumentKind::Identity,
            HiveCrewDocumentKind::Soul,
            HiveCrewDocumentKind::Memory,
        ] {
            let file_name = kind.preferred_file_name();
            if create_document_if_missing(&crew_dir, file_name, &kind.default_content(slug))? {
                created_files.push(format!("crew/{slug}/{file_name}"));
            }
        }
    }

    Ok(HiveBootstrapResult {
        created_files,
        profile: HiveHomeProfile::load_from(hive_home),
    })
}

pub fn write_hive_home_document(
    hive_home: &Path,
    kind: HiveHomeDocumentKind,
    content: &str,
) -> std::io::Result<HiveHomeDocument> {
    fs::create_dir_all(hive_home)?;
    write_document(hive_home.join(kind.preferred_file_name()), content)
}

pub fn write_hive_crew_document(
    hive_home: &Path,
    slug: &str,
    kind: HiveCrewDocumentKind,
    content: &str,
) -> std::io::Result<HiveHomeDocument> {
    let crew_dir = hive_home.join("crew").join(slug);
    fs::create_dir_all(&crew_dir)?;
    write_document(crew_dir.join(kind.preferred_file_name()), content)
}

pub fn is_valid_crew_slug(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 64
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
}

fn create_document_if_missing(dir: &Path, file_name: &str, content: &str) -> std::io::Result<bool> {
    let path = dir.join(file_name);
    if path.exists() {
        return Ok(false);
    }
    write_document(path, content)?;
    Ok(true)
}

fn write_document(path: PathBuf, content: &str) -> std::io::Result<HiveHomeDocument> {
    let trimmed = content.trim();
    fs::write(&path, trimmed)?;
    Ok(HiveHomeDocument {
        file_name: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        content: trimmed.to_string(),
    })
}
