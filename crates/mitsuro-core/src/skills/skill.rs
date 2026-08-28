//! Skill data structures and Agent Skills-compatible validation.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

/// Where a discovered skill is scoped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillSource {
    /// User-level skills (for example `~/.mitsuro/skills` or `~/.agents/skills`).
    Global,
    /// A skill discovered from the current project/worktree.
    Project,
    /// A skill root registered by an installed package/plugin.
    Package,
}

impl SkillSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
            Self::Package => "package",
        }
    }
}

/// Invocation policy for one skill.
///
/// Skills only load instructions; any tools those instructions cause the agent
/// to use remain governed by the inherited `ToolContext`. `Ask` therefore means
/// the instructions may only be loaded from a supervised parent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SkillPermission {
    #[default]
    Allow,
    Ask,
    Deny,
}

impl SkillPermission {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Allow => Self::Ask,
            Self::Ask => Self::Deny,
            Self::Deny => Self::Allow,
        }
    }
}

impl fmt::Display for SkillPermission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Skill metadata for listing/discovery (lightweight).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub author: Option<String>,
    pub tags: Vec<String>,
    pub source: SkillSource,
    /// Human-readable discovery root (`mitsuro`, `agents`, `pi`, `package:foo`, ...).
    pub origin: String,
    /// Canonical definition path, useful for diagnostics and explicit invocation.
    pub path: PathBuf,
    /// Whether the skill is enabled by local policy.
    pub enabled: bool,
    /// Effective local invocation policy.
    pub permission: SkillPermission,
    /// Whether the skill should be advertised to the model automatically.
    pub model_invocable: bool,
}

/// YAML frontmatter from `SKILL.md`.
///
/// The Agent Skills fields are supported alongside Mitsuro's existing optional
/// catalog fields. Unknown fields intentionally remain forward-compatible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub compatibility: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default, rename = "allowed-tools")]
    pub allowed_tools: Option<String>,
    #[serde(default, rename = "disable-model-invocation")]
    pub disable_model_invocation: bool,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Full skill with content loaded.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub allowed_tools: Option<String>,
    pub disable_model_invocation: bool,
    pub version: Option<String>,
    pub author: Option<String>,
    pub tags: Vec<String>,
    pub source: SkillSource,
    /// Human-readable discovery root.
    pub origin: String,
    /// Path to the skill directory.
    pub path: PathBuf,
    /// Exact `SKILL.md` path.
    pub definition_path: PathBuf,
    /// Effective policy state, filled by `SkillsManager`.
    pub enabled: bool,
    pub permission: SkillPermission,
    /// `SKILL.md` content with frontmatter removed.
    pub content: String,
}

impl Skill {
    /// Parse `SKILL.md` content into a skill.
    ///
    /// Directory-name validation is performed by the filesystem loader because
    /// callers of this compatibility constructor may provide synthetic paths.
    pub fn parse(content: &str, path: PathBuf, source: SkillSource) -> Result<Self> {
        Self::parse_with_origin(content, path, source, source.as_str().to_string())
    }

    pub(crate) fn parse_with_origin(
        content: &str,
        path: PathBuf,
        source: SkillSource,
        origin: String,
    ) -> Result<Self> {
        let definition_path = path.join("SKILL.md");
        Self::parse_definition_with_origin(content, path, definition_path, source, origin)
    }

    pub(crate) fn parse_definition_with_origin(
        content: &str,
        path: PathBuf,
        definition_path: PathBuf,
        source: SkillSource,
        origin: String,
    ) -> Result<Self> {
        let (frontmatter, body) = parse_frontmatter(content)?;

        Ok(Self {
            name: frontmatter.name,
            description: frontmatter.description,
            license: frontmatter.license,
            compatibility: frontmatter.compatibility,
            metadata: frontmatter.metadata,
            allowed_tools: frontmatter.allowed_tools,
            disable_model_invocation: frontmatter.disable_model_invocation,
            version: frontmatter.version,
            author: frontmatter.author,
            tags: frontmatter.tags,
            source,
            origin,
            path,
            definition_path,
            enabled: true,
            permission: SkillPermission::Allow,
            content: body,
        })
    }

    /// Convert to lightweight `SkillInfo`.
    pub fn to_info(&self) -> SkillInfo {
        SkillInfo {
            name: self.name.clone(),
            description: self.description.clone(),
            version: self.version.clone(),
            author: self.author.clone(),
            tags: self.tags.clone(),
            source: self.source,
            origin: self.origin.clone(),
            path: self.definition_path.clone(),
            enabled: self.enabled,
            permission: self.permission,
            model_invocable: self.is_model_invocable(),
        }
    }

    /// Whether this skill may be advertised for model-driven disclosure.
    ///
    /// `Ask` stays invocable; only `Deny` is excluded. Load-time supervision
    /// for `Ask` stays on the tool/user path, not this predicate.
    pub(crate) fn is_model_invocable(&self) -> bool {
        self.enabled && self.permission != SkillPermission::Deny && !self.disable_model_invocation
    }

    pub fn get_content(&self) -> &str {
        &self.content
    }
}

/// Validate a skill name according to the Agent Skills specification.
pub fn validate_skill_name(name: &str) -> Result<()> {
    let len = name.chars().count();
    if !(1..=64).contains(&len) {
        return Err(anyhow!("Skill name must be between 1 and 64 characters"));
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return Err(anyhow!(
            "Skill name must use single hyphens only and cannot start or end with a hyphen"
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(anyhow!(
            "Skill name must contain only lowercase ASCII letters, numbers, and hyphens"
        ));
    }
    Ok(())
}

/// Parse YAML frontmatter from markdown content.
fn parse_frontmatter(content: &str) -> Result<(SkillFrontmatter, String)> {
    // Preserve body whitespace while accepting an optional UTF-8 BOM.
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let Some(rest) = content.strip_prefix("---") else {
        return Err(anyhow!("SKILL.md must start with YAML frontmatter (---)"));
    };
    if !rest.starts_with('\n') && !rest.starts_with("\r\n") {
        return Err(anyhow!(
            "Opening frontmatter delimiter must be on its own line"
        ));
    }

    let normalized = rest.strip_prefix("\r\n").unwrap_or_else(|| &rest[1..]);
    let mut offset = 0usize;
    let mut closing = None;
    for line in normalized.split_inclusive('\n') {
        let candidate = line.trim_end_matches(['\r', '\n']);
        if candidate == "---" {
            closing = Some((offset, offset + line.len()));
            break;
        }
        offset += line.len();
    }
    // Also support a closing delimiter at EOF.
    if closing.is_none() && normalized[offset..].trim_end_matches('\r') == "---" {
        closing = Some((offset, normalized.len()));
    }
    let (yaml_end, body_start) =
        closing.ok_or_else(|| anyhow!("Missing closing frontmatter delimiter (---)"))?;

    let frontmatter: SkillFrontmatter = serde_yaml::from_str(&normalized[..yaml_end])
        .map_err(|e| anyhow!("Failed to parse SKILL.md frontmatter: {e}"))?;

    validate_skill_name(&frontmatter.name)?;
    let description_len = frontmatter.description.trim().chars().count();
    if !(1..=1024).contains(&description_len) {
        return Err(anyhow!(
            "Skill description must be between 1 and 1024 characters"
        ));
    }
    if frontmatter
        .compatibility
        .as_ref()
        .is_some_and(|value| value.chars().count() > 500)
    {
        return Err(anyhow!(
            "Skill compatibility must be at most 500 characters"
        ));
    }

    Ok((
        frontmatter,
        normalized[body_start..]
            .trim_start_matches(['\r', '\n'])
            .trim_end()
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_skills_frontmatter() {
        let content = r#"---
name: git-commit
description: Generate descriptive commit messages
license: MIT
compatibility: Requires git
metadata:
  category: workflow
allowed-tools: read bash
disable-model-invocation: true
version: 1.0.0
author: mitsuro
tags: [git, workflow]
---

# Git Commit Helper
"#;

        let skill = Skill::parse(content, PathBuf::from("/test"), SkillSource::Global).unwrap();
        assert_eq!(skill.name, "git-commit");
        assert_eq!(skill.license.as_deref(), Some("MIT"));
        assert_eq!(
            skill.metadata.get("category").map(String::as_str),
            Some("workflow")
        );
        assert_eq!(skill.allowed_tools.as_deref(), Some("read bash"));
        assert!(skill.disable_model_invocation);
        assert!(skill.content.contains("Git Commit Helper"));
    }

    #[test]
    fn parses_crlf_frontmatter() {
        let content = "---\r\nname: simple\r\ndescription: A simple skill\r\n---\r\nBody\r\n";
        let skill = Skill::parse(content, PathBuf::from("/test"), SkillSource::Project).unwrap();
        assert_eq!(skill.content, "Body");
    }

    #[test]
    fn rejects_invalid_standard_names() {
        for name in ["Invalid", "-invalid", "invalid-", "invalid--name", ""] {
            assert!(validate_skill_name(name).is_err(), "accepted {name:?}");
        }
        assert!(validate_skill_name(&"a".repeat(65)).is_err());
        assert!(validate_skill_name("valid-skill-2").is_ok());
    }

    #[test]
    fn rejects_description_over_standard_limit() {
        let content = format!("---\nname: long\ndescription: {}\n---\n", "x".repeat(1025));
        assert!(Skill::parse(&content, PathBuf::from("/test"), SkillSource::Global).is_err());
    }

    #[test]
    fn missing_frontmatter_is_rejected() {
        assert!(Skill::parse(
            "# No frontmatter",
            PathBuf::from("/test"),
            SkillSource::Global
        )
        .is_err());
    }
}
