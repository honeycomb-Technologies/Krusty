use chrono::{DateTime, Utc};

/// File access activity for importance scoring
#[derive(Debug, Clone)]
pub struct FileActivity {
    pub file_path: String,
    pub read_count: usize,
    pub write_count: usize,
    pub edit_count: usize,
    pub last_accessed: DateTime<Utc>,
    pub user_referenced: bool,
}

impl FileActivity {
    /// Calculate importance score (higher = more important)
    ///
    /// Weights:
    /// - Writes: 3 points each (created/overwritten files are critical)
    /// - Edits: 2 points each (modified files are important)
    /// - Reads: 1 point each (viewed files provide context)
    /// - User reference: 5 point bonus (explicitly mentioned by user)
    /// - Recency: multiplier based on how recently accessed
    pub fn importance_score(&self, now: DateTime<Utc>) -> f64 {
        let activity_score = (self.write_count * 3 + self.edit_count * 2 + self.read_count) as f64;
        let user_bonus = if self.user_referenced { 5.0 } else { 0.0 };

        let hours_ago = (now - self.last_accessed).num_hours().max(0) as f64;
        let recency_multiplier = 1.0 / (1.0 + hours_ago / 24.0);

        (activity_score + user_bonus) * (0.5 + 0.5 * recency_multiplier)
    }
}

/// A file ranked by importance with reasons
#[derive(Debug, Clone)]
pub struct RankedFile {
    pub path: String,
    pub score: f64,
    pub reasons: Vec<String>,
}

impl RankedFile {
    /// Create from file activity
    pub fn from_activity(activity: &FileActivity, now: DateTime<Utc>) -> Self {
        Self {
            path: activity.file_path.clone(),
            score: activity.importance_score(now),
            reasons: build_activity_reasons(
                activity.read_count as i64,
                activity.write_count as i64,
                activity.edit_count as i64,
                activity.user_referenced,
            ),
        }
    }
}

pub(super) fn build_activity_reasons(
    read_count: i64,
    write_count: i64,
    edit_count: i64,
    user_referenced: bool,
) -> Vec<String> {
    let mut reasons = Vec::with_capacity(4);

    if write_count > 0 {
        reasons.push(format!("written {} time(s)", write_count));
    }
    if edit_count > 0 {
        reasons.push(format!("edited {} time(s)", edit_count));
    }
    if read_count > 0 {
        reasons.push(format!("read {} time(s)", read_count));
    }
    if user_referenced {
        reasons.push("referenced by user".to_string());
    }

    reasons
}
