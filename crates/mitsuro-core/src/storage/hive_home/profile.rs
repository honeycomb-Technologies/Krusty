use std::fs;
use std::path::{Path, PathBuf};

use tracing::warn;

use crate::paths;

use super::model::{
    CHANNELS_CANDIDATES, CREW_IDENTITY_CANDIDATES, CREW_MEMORY_CANDIDATES, CREW_SOUL_CANDIDATES,
    HEARTBEAT_CANDIDATES, IDENTITY_CANDIDATES, MEMORY_CANDIDATES, SOUL_CANDIDATES, USER_CANDIDATES,
};
use super::{HiveContextLayer, HiveCrewProfile, HiveHomeDocument, HiveHomeProfile};

impl HiveHomeProfile {
    pub fn load() -> Self {
        Self::load_from(&paths::hive_dir())
    }

    pub fn load_from(hive_home: &Path) -> Self {
        Self {
            soul: load_named_document(hive_home, SOUL_CANDIDATES, "Hive soul"),
            identity: load_named_document(hive_home, IDENTITY_CANDIDATES, "Hive identity"),
            user: load_named_document(hive_home, USER_CANDIDATES, "Hive user model"),
            heartbeat: load_named_document(hive_home, HEARTBEAT_CANDIDATES, "Hive heartbeat"),
            memory: load_named_document(hive_home, MEMORY_CANDIDATES, "Hive memory"),
            channels: load_named_document(hive_home, CHANNELS_CANDIDATES, "Hive channels"),
            crew: load_crew_profiles(&hive_home.join("crew")),
        }
    }

    pub fn context_layers(&self) -> Vec<HiveContextLayer> {
        let mut layers = Vec::new();
        push_layer(&mut layers, "SOUL", self.soul.clone());
        push_layer(&mut layers, "IDENTITY", self.identity.clone());
        push_layer(&mut layers, "USER", self.user.clone());
        push_layer(&mut layers, "HEARTBEAT", self.heartbeat.clone());
        push_layer(&mut layers, "CHANNELS", self.channels.clone());
        layers
    }
}

fn push_layer(
    layers: &mut Vec<HiveContextLayer>,
    kind: &'static str,
    document: Option<HiveHomeDocument>,
) {
    if let Some(document) = document {
        layers.push(HiveContextLayer { kind, document });
    }
}

fn load_crew_profiles(crew_root: &Path) -> Vec<HiveCrewProfile> {
    let entries = match fs::read_dir(crew_root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut dirs = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            path.is_dir().then_some(path)
        })
        .collect::<Vec<_>>();
    dirs.sort();

    dirs.into_iter()
        .filter_map(|dir| {
            let slug = dir
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.to_string())?;

            let profile = HiveCrewProfile {
                slug,
                identity: load_named_document(&dir, CREW_IDENTITY_CANDIDATES, "Hive crew identity"),
                soul: load_named_document(&dir, CREW_SOUL_CANDIDATES, "Hive crew soul"),
                memory: load_named_document(&dir, CREW_MEMORY_CANDIDATES, "Hive crew memory"),
            };

            if profile.identity.is_none() && profile.soul.is_none() && profile.memory.is_none() {
                None
            } else {
                Some(profile)
            }
        })
        .collect()
}

fn load_named_document(
    base_dir: &Path,
    candidates: &[&str],
    context: &'static str,
) -> Option<HiveHomeDocument> {
    let path = discover_named_file(base_dir, candidates)?;
    load_document(&path, context)
}

fn discover_named_file(base_dir: &Path, candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(|name| base_dir.join(name))
        .find(|path| path.is_file())
}

fn load_document(path: &Path, context: &'static str) -> Option<HiveHomeDocument> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            warn!(context, path = %path.display(), error = %error, "Failed to read Hive home document");
            return None;
        }
    };

    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(HiveHomeDocument {
        file_name: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        content: trimmed.to_string(),
    })
}
