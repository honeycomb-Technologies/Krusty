//! Skills manager - central coordinator for skill discovery and loading

use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::info;

use super::loader::{
    ensure_skills_dir, load_skill, load_skill_file, load_skills_from_dir, scaffold_skill,
};
use super::skill::{Skill, SkillInfo, SkillSource};

/// Manages skill discovery, loading, and access
pub struct SkillsManager {
    /// Global skills directory (~/.krusty/skills/)
    global_dir: PathBuf,
    /// Project-specific skills directory (.krusty/skills/)
    project_dir: Option<PathBuf>,
    /// Cached skills (name -> Skill)
    cache: HashMap<String, Skill>,
    /// Whether cache is populated
    cache_valid: bool,
}

impl SkillsManager {
    /// Create a new SkillsManager
    ///
    /// # Arguments
    /// * `global_dir` - Path to global skills directory (~/.krusty/skills/)
    /// * `project_dir` - Optional path to project-specific skills (.krusty/skills/)
    ///
    /// # Example
    /// ```no_run
    /// use krusty_core::skills::SkillsManager;
    /// use std::path::PathBuf;
    ///
    /// let manager = SkillsManager::new(
    ///     PathBuf::from("~/.krusty/skills"),
    ///     Some(PathBuf::from("/project/.krusty/skills"))
    /// );
    /// ```
    pub fn new(global_dir: PathBuf, project_dir: Option<PathBuf>) -> Self {
        Self {
            global_dir,
            project_dir,
            cache: HashMap::new(),
            cache_valid: false,
        }
    }

    /// Create with default directories
    pub fn with_defaults(working_dir: &std::path::Path) -> Self {
        let global_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".krusty")
            .join("skills");

        let project_dir = working_dir.join(".krusty").join("skills");
        let project_dir = if project_dir.exists() {
            Some(project_dir)
        } else {
            None
        };

        Self::new(global_dir, project_dir)
    }

    /// Ensure the global skills directory exists
    pub fn ensure_global_dir(&self) -> Result<()> {
        ensure_skills_dir(&self.global_dir)
    }

    /// Refresh the skills cache
    pub fn refresh(&mut self) {
        self.cache.clear();

        // Load project skills first (highest priority)
        if let Some(ref project_dir) = self.project_dir {
            for skill in load_skills_from_dir(project_dir, SkillSource::Project) {
                self.cache.insert(skill.name.clone(), skill);
            }
        }

        // Load global skills (don't override project skills)
        for skill in load_skills_from_dir(&self.global_dir, SkillSource::Global) {
            self.cache.entry(skill.name.clone()).or_insert(skill);
        }

        self.cache_valid = true;
        info!("Loaded {} skills", self.cache.len());
    }

    /// Ensure cache is populated
    fn ensure_cache(&mut self) {
        if !self.cache_valid {
            self.refresh();
        }
    }

    /// List all available skills (metadata only)
    ///
    /// Returns sorted list of skill metadata from both global and project directories.
    /// Project skills override global skills with the same name.
    ///
    /// # Example
    /// ```no_run
    /// # use krusty_core::skills::SkillsManager;
    /// # let mut manager = SkillsManager::with_defaults(".".as_ref());
    /// let skills = manager.list_skills();
    /// for skill in skills {
    ///     println!("{}: {}", skill.name, skill.description);
    /// }
    /// ```
    pub fn list_skills(&mut self) -> Vec<SkillInfo> {
        self.ensure_cache();
        let mut skills: Vec<SkillInfo> = self.cache.values().map(|s| s.to_info()).collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills
    }

    /// Get a skill by name
    ///
    /// Returns reference to cached skill if it exists.
    /// Project skills take precedence over global skills.
    ///
    /// # Example
    /// ```no_run
    /// # use krusty_core::skills::SkillsManager;
    /// # let mut manager = SkillsManager::with_defaults(".".as_ref());
    /// if let Some(skill) = manager.get_skill("rust") {
    ///     println!("Found rust skill: {}", skill.description);
    /// }
    /// ```
    pub fn get_skill(&mut self, name: &str) -> Option<&Skill> {
        self.ensure_cache();
        self.cache.get(name)
    }

    /// Check if a skill exists
    pub fn skill_exists(&mut self, name: &str) -> bool {
        self.ensure_cache();
        self.cache.contains_key(name)
    }

    /// Load skill content (main SKILL.md body)
    pub fn load_skill_content(&mut self, name: &str) -> Result<String> {
        self.ensure_cache();
        self.cache
            .get(name)
            .map(|s| s.content.clone())
            .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found", name))
    }

    /// Load a file from within a skill
    pub fn load_file_from_skill(&mut self, skill_name: &str, file: &str) -> Result<String> {
        self.ensure_cache();
        let skill = self
            .cache
            .get(skill_name)
            .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found", skill_name))?;

        load_skill_file(&skill.path, file)
    }

    /// Get skills metadata formatted for system prompt
    pub fn get_skills_metadata(&mut self) -> String {
        self.ensure_cache();

        if self.cache.is_empty() {
            return String::new();
        }

        let mut lines = Vec::new();
        for skill in self.cache.values() {
            lines.push(format!("- **{}**: {}", skill.name, skill.description));
        }
        lines.sort();
        lines.join("\n")
    }

    /// Create a new skill in the global directory
    pub fn create_skill(&mut self, name: &str, description: &str) -> Result<PathBuf> {
        ensure_skills_dir(&self.global_dir)?;
        let path = scaffold_skill(&self.global_dir, name, description)?;
        self.cache_valid = false; // Invalidate cache
        Ok(path)
    }

    /// Delete a skill
    pub fn delete_skill(&mut self, name: &str) -> Result<()> {
        self.ensure_cache();

        let skill = self
            .cache
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found", name))?;

        // Only allow deleting from global dir (project skills are managed differently)
        if skill.source != SkillSource::Global {
            return Err(anyhow::anyhow!(
                "Can only delete global skills. Project skills should be managed via version control."
            ));
        }

        std::fs::remove_dir_all(&skill.path)?;
        self.cache_valid = false;
        Ok(())
    }

    /// Reload a specific skill (useful after editing)
    pub fn reload_skill(&mut self, name: &str) -> Result<()> {
        let skill = self
            .cache
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found", name))?;

        let path = skill.path.clone();
        let source = skill.source;

        let reloaded = load_skill(&path, source)?;
        self.cache.insert(name.to_string(), reloaded);
        Ok(())
    }

    /// Get the global skills directory path
    pub fn global_dir(&self) -> &PathBuf {
        &self.global_dir
    }

    /// Get the project skills directory path
    pub fn project_dir(&self) -> Option<&PathBuf> {
        self.project_dir.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_skills_manager() {
        let temp = tempdir().unwrap();
        let global_dir = temp.path().join("global");
        let project_dir = temp.path().join("project");

        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::create_dir_all(&project_dir).unwrap();

        // Create a test skill
        let skill_dir = global_dir.join("test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test-skill\ndescription: A test\n---\n# Test",
        )
        .unwrap();

        let mut manager = SkillsManager::new(global_dir, Some(project_dir));

        let skills = manager.list_skills();
        assert_eq!(skills.len(), 1);
        assert!(skills.iter().any(|s| s.name == "test-skill"));

        assert!(manager.skill_exists("test-skill"));
        assert!(!manager.skill_exists("nonexistent"));
    }

    #[test]
    fn test_project_overrides_global() {
        let temp = tempdir().unwrap();
        let global_dir = temp.path().join("global");
        let project_dir = temp.path().join("project");

        // Create same skill in both directories
        for (dir, desc) in [
            (&global_dir, "Global version"),
            (&project_dir, "Project version"),
        ] {
            let skill_dir = dir.join("shared-skill");
            std::fs::create_dir_all(&skill_dir).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: shared-skill\ndescription: {}\n---\n", desc),
            )
            .unwrap();
        }

        let mut manager = SkillsManager::new(global_dir, Some(project_dir));
        let skill = manager.get_skill("shared-skill").unwrap();

        // Project version should take precedence
        assert_eq!(skill.description, "Project version");
        assert_eq!(skill.source, SkillSource::Project);
    }
}
