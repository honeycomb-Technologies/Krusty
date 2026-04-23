use std::fs;
use std::path::{Path, PathBuf};

use super::model::DEFAULT_CREW_SLUGS;
use super::{
    MakoBootstrapResult, MakoCrewDocumentKind, MakoHomeDocument, MakoHomeDocumentKind,
    MakoHomeProfile,
};

pub fn bootstrap_mako_home(mako_home: &Path) -> std::io::Result<MakoBootstrapResult> {
    fs::create_dir_all(mako_home)?;

    let mut created_files = Vec::new();
    for kind in [
        MakoHomeDocumentKind::Soul,
        MakoHomeDocumentKind::Identity,
        MakoHomeDocumentKind::Heartbeat,
        MakoHomeDocumentKind::Memory,
        MakoHomeDocumentKind::Channels,
    ] {
        if create_document_if_missing(
            mako_home,
            kind.preferred_file_name(),
            kind.default_content(),
        )? {
            created_files.push(kind.preferred_file_name().to_string());
        }
    }

    for slug in DEFAULT_CREW_SLUGS {
        let crew_dir = mako_home.join("crew").join(slug);
        fs::create_dir_all(&crew_dir)?;
        for kind in [
            MakoCrewDocumentKind::Identity,
            MakoCrewDocumentKind::Soul,
            MakoCrewDocumentKind::Memory,
        ] {
            let file_name = kind.preferred_file_name();
            if create_document_if_missing(&crew_dir, file_name, &kind.default_content(slug))? {
                created_files.push(format!("crew/{slug}/{file_name}"));
            }
        }
    }

    Ok(MakoBootstrapResult {
        created_files,
        profile: MakoHomeProfile::load_from(mako_home),
    })
}

pub fn write_mako_home_document(
    mako_home: &Path,
    kind: MakoHomeDocumentKind,
    content: &str,
) -> std::io::Result<MakoHomeDocument> {
    fs::create_dir_all(mako_home)?;
    write_document(mako_home.join(kind.preferred_file_name()), content)
}

pub fn write_mako_crew_document(
    mako_home: &Path,
    slug: &str,
    kind: MakoCrewDocumentKind,
    content: &str,
) -> std::io::Result<MakoHomeDocument> {
    let crew_dir = mako_home.join("crew").join(slug);
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

fn write_document(path: PathBuf, content: &str) -> std::io::Result<MakoHomeDocument> {
    let trimmed = content.trim();
    fs::write(&path, trimmed)?;
    Ok(MakoHomeDocument {
        file_name: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        content: trimmed.to_string(),
    })
}
