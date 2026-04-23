use std::path::Path;

use tracing::warn;

use crate::paths;
use crate::storage::MakoHomeProfile;

use super::project::discover_named_file;

const MAKO_FILES: &[&str] = &["MAKO.md", "mako.md"];

pub(super) fn build_mako_context_sections(
    project_root: &Path,
    mako_crew_slug: Option<&str>,
) -> Vec<String> {
    let mako_home = paths::mako_dir();
    build_mako_context_sections_with_home(project_root, &mako_home, mako_crew_slug)
}

pub(super) fn build_mako_context_sections_with_home(
    project_root: &Path,
    mako_home: &Path,
    mako_crew_slug: Option<&str>,
) -> Vec<String> {
    let profile = MakoHomeProfile::load_from(mako_home);
    let mut sections = profile
        .context_layers()
        .into_iter()
        .map(|layer| {
            format!(
                "[MAKO {} - {}]\n\n{}\n\n[END MAKO {}]",
                layer.kind, layer.document.file_name, layer.document.content, layer.kind
            )
        })
        .collect::<Vec<_>>();

    if let Some(crew_slug) = mako_crew_slug
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(member) = profile.crew.iter().find(|member| member.slug == crew_slug) {
            for (kind, document) in [
                ("CREW IDENTITY", member.identity.as_ref()),
                ("CREW SOUL", member.soul.as_ref()),
                ("CREW MEMORY", member.memory.as_ref()),
            ] {
                if let Some(document) = document {
                    sections.push(format!(
                        "[MAKO {} - {} - {}]\n\n{}\n\n[END MAKO {}]",
                        kind, member.slug, document.file_name, document.content, kind
                    ));
                }
            }
        }
    }

    if let Some(path) = discover_named_file(project_root, MAKO_FILES) {
        if let Some(content) = load_mako_context_file(&path, "Mako project overlay") {
            let label = display_context_file_name(&path, "MAKO.md");
            sections.push(format!(
                "[MAKO PROJECT OVERLAY - {}]\n\n{}\n\n[END MAKO PROJECT OVERLAY]",
                label, content
            ));
        }
    }

    sections
}

fn load_mako_context_file(path: &Path, context: &'static str) -> Option<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            warn!(context, path = %path.display(), error = %error, "Failed to read Mako context file");
            return None;
        }
    };

    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

fn display_context_file_name(path: &Path, fallback: &str) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(fallback)
        .to_string()
}
