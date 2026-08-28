//! Server-side mention parsing for group rooms.
//!
//! Mentions route a group turn to selected members: `@slug` matches exactly,
//! `@Display Name` matches a Worker's spaced display name, `@all` (or no
//! mention at all) targets every member. Exact slugs and full display names
//! win; a unique slug prefix is accepted as a convenience, while ambiguous or
//! unknown mentions fail closed instead of broadening the audience.

use std::fmt;

/// One roster entry a mention can resolve to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMentionTarget {
    pub worker_id: String,
    pub slug: String,
    pub display_name: String,
}

/// One shorthand mention that matched more than one Worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguousGroupMention {
    pub mention: String,
    pub candidate_slugs: Vec<String>,
}

/// A mention routing error that must be corrected before a group turn starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MentionResolutionError {
    Ambiguous {
        mention: String,
        candidate_slugs: Vec<String>,
    },
    Unresolved {
        mentions: Vec<String>,
    },
}

impl fmt::Display for MentionResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ambiguous {
                mention,
                candidate_slugs,
            } => write!(
                formatter,
                "mention '@{mention}' is ambiguous; use one of: {}",
                candidate_slugs
                    .iter()
                    .map(|slug| format!("@{slug}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Unresolved { mentions } => write!(
                formatter,
                "unknown group {}: {}",
                if mentions.len() == 1 {
                    "mention"
                } else {
                    "mentions"
                },
                mentions
                    .iter()
                    .map(|mention| format!("@{mention}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl std::error::Error for MentionResolutionError {}

/// Outcome of scanning one message against a roster.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MentionResolution {
    /// Whether the message contained any `@` mention token at all.
    pub saw_mention: bool,
    /// `@all` was present.
    pub mentions_all: bool,
    /// Explicitly resolved Worker ids in roster order, deduplicated.
    pub explicit_worker_ids: Vec<String>,
    /// Shorthand mentions that matched more than one roster slug.
    pub ambiguous: Vec<AmbiguousGroupMention>,
    /// Mention tokens that matched no roster entry.
    pub unresolved: Vec<String>,
}

impl MentionResolution {
    /// Effective turn targets. Invalid explicit mentions fail before the
    /// caller persists or dispatches a group turn.
    pub fn resolve_targets(
        &self,
        roster: &[GroupMentionTarget],
    ) -> Result<Vec<String>, MentionResolutionError> {
        if let Some(ambiguous) = self.ambiguous.first() {
            return Err(MentionResolutionError::Ambiguous {
                mention: ambiguous.mention.clone(),
                candidate_slugs: ambiguous.candidate_slugs.clone(),
            });
        }
        if !self.unresolved.is_empty() {
            return Err(MentionResolutionError::Unresolved {
                mentions: self.unresolved.clone(),
            });
        }
        if self.mentions_all || !self.saw_mention {
            return Ok(roster
                .iter()
                .map(|target| target.worker_id.clone())
                .collect());
        }
        Ok(self.explicit_worker_ids.clone())
    }
}

/// Scan `content` for mentions against `roster`.
pub fn parse_group_mentions(content: &str, roster: &[GroupMentionTarget]) -> MentionResolution {
    let mut resolution = MentionResolution::default();
    let mut matched_ids: Vec<String> = Vec::new();
    let bytes = content.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'@' || !starts_mention(content, index) {
            index += 1;
            continue;
        }
        let after = &content[index + 1..];
        // Longest match wins so "@Deep Researcher" resolves to the spaced
        // display name instead of stopping at the token "Deep".
        let token = leading_mention_token(after);
        if token.is_empty() {
            index += 1;
            continue;
        }
        resolution.saw_mention = true;
        let token_len = token.len();
        let token = token.to_ascii_lowercase();
        if token == "all" {
            resolution.mentions_all = true;
        } else {
            let display_match = longest_display_name_matches(after, roster);
            if let Some((display_candidates, consumed)) = display_match.as_ref() {
                if *consumed > token_len {
                    let display_mention = after[..*consumed].to_ascii_lowercase();
                    record_candidates(
                        &display_mention,
                        display_candidates,
                        &mut matched_ids,
                        &mut resolution,
                    );
                    index += 1 + consumed;
                    continue;
                }
            }

            let mut exact = roster
                .iter()
                .filter(|target| target.slug == token)
                .collect::<Vec<_>>();
            if let Some((display_candidates, _)) = display_match {
                for candidate in display_candidates {
                    if !exact
                        .iter()
                        .any(|target| target.worker_id == candidate.worker_id)
                    {
                        exact.push(candidate);
                    }
                }
            }
            let candidates = if exact.is_empty() {
                roster
                    .iter()
                    .filter(|target| target.slug.starts_with(&token))
                    .collect::<Vec<_>>()
            } else {
                exact
            };
            if candidates.is_empty() {
                resolution.unresolved.push(token);
            } else {
                record_candidates(&token, &candidates, &mut matched_ids, &mut resolution);
            }
        }
        index += 1 + token_len;
    }

    // Keep explicit targets in roster order and deduplicated so fan-out and
    // speaker plans stay deterministic.
    resolution.explicit_worker_ids = roster
        .iter()
        .filter(|target| matched_ids.contains(&target.worker_id))
        .map(|target| target.worker_id.clone())
        .collect();
    resolution
}

fn record_candidates(
    mention: &str,
    candidates: &[&GroupMentionTarget],
    matched_ids: &mut Vec<String>,
    resolution: &mut MentionResolution,
) {
    if candidates.len() == 1 {
        matched_ids.push(candidates[0].worker_id.clone());
    } else {
        resolution.ambiguous.push(AmbiguousGroupMention {
            mention: mention.to_string(),
            candidate_slugs: candidates
                .iter()
                .map(|target| target.slug.clone())
                .collect(),
        });
    }
}

/// A mention starts at the beginning of the text or after a non-word
/// character, so `user@example.com` never mentions `@example`.
fn starts_mention(content: &str, at: usize) -> bool {
    content[..at]
        .chars()
        .next_back()
        .map(|previous| !previous.is_alphanumeric() && previous != '@')
        .unwrap_or(true)
}

fn leading_mention_token(after: &str) -> &str {
    let end = after
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(after.len());
    &after[..end]
}

/// Case-insensitive spaced display-name match followed by a word boundary.
fn longest_display_name_matches<'roster>(
    after: &str,
    roster: &'roster [GroupMentionTarget],
) -> Option<(Vec<&'roster GroupMentionTarget>, usize)> {
    let mut best: Vec<&GroupMentionTarget> = Vec::new();
    let mut best_length = 0usize;
    for target in roster {
        let name = target.display_name.trim();
        if name.is_empty() || after.len() < name.len() {
            continue;
        }
        let Some(candidate) = after.get(..name.len()) else {
            continue;
        };
        if !candidate.eq_ignore_ascii_case(name) {
            continue;
        }
        let boundary_ok = after[name.len()..]
            .chars()
            .next()
            .map(|next| !next.is_alphanumeric())
            .unwrap_or(true);
        if !boundary_ok {
            continue;
        }
        if name.len() > best_length {
            best.clear();
            best.push(target);
            best_length = name.len();
        } else if name.len() == best_length {
            best.push(target);
        }
    }
    (!best.is_empty()).then_some((best, best_length))
}

#[cfg(test)]
mod tests {
    use super::{parse_group_mentions, GroupMentionTarget, MentionResolutionError};

    fn roster() -> Vec<GroupMentionTarget> {
        vec![
            GroupMentionTarget {
                worker_id: "w-researcher".into(),
                slug: "researcher".into(),
                display_name: "Deep Researcher".into(),
            },
            GroupMentionTarget {
                worker_id: "w-reviewer".into(),
                slug: "reviewer".into(),
                display_name: "Reviewer".into(),
            },
            GroupMentionTarget {
                worker_id: "w-builder".into(),
                slug: "builder".into(),
                display_name: "Builder".into(),
            },
        ]
    }

    fn target_ids(content: &str) -> Result<Vec<String>, MentionResolutionError> {
        parse_group_mentions(content, &roster()).resolve_targets(&roster())
    }

    #[test]
    fn mention_routing_table() {
        let all = vec![
            "w-researcher".to_string(),
            "w-reviewer".to_string(),
            "w-builder".to_string(),
        ];
        let cases: Vec<(&str, Vec<String>)> = vec![
            // No mention targets everyone.
            ("please look at the release", all.clone()),
            // @all targets everyone.
            ("@all sync up on the plan", all.clone()),
            // Exact slug.
            ("@builder ship the fix", vec!["w-builder".into()]),
            // Spaced full display name.
            (
                "@Deep Researcher dig into this",
                vec!["w-researcher".into()],
            ),
            // Display name match is case-insensitive.
            (
                "@deep researcher dig into this",
                vec!["w-researcher".into()],
            ),
            // A unique short prefix remains a convenient exact routing hint.
            ("@b ship the fix", vec!["w-builder".into()]),
            // Multiple mentions combine in roster order without duplicates.
            (
                "@builder and @researcher and @builder again",
                vec!["w-researcher".into(), "w-builder".into()],
            ),
            // Emails never mention.
            ("mail bob@researcher.dev instead", all),
            // Mid-sentence mention after punctuation resolves.
            ("cc: @reviewer, thanks", vec!["w-reviewer".into()]),
        ];
        for (content, expected) in cases {
            assert_eq!(target_ids(content).unwrap(), expected, "content: {content}");
        }
    }

    #[test]
    fn ambiguous_prefix_fails_with_candidate_slugs() {
        assert_eq!(
            target_ids("@re check the diff").unwrap_err(),
            MentionResolutionError::Ambiguous {
                mention: "re".into(),
                candidate_slugs: vec!["researcher".into(), "reviewer".into()],
            }
        );
    }

    #[test]
    fn unresolved_mention_fails_instead_of_targeting_everyone() {
        assert_eq!(
            target_ids("@nobody around?").unwrap_err(),
            MentionResolutionError::Unresolved {
                mentions: vec!["nobody".into()],
            }
        );
    }

    #[test]
    fn resolution_reports_mention_metadata() {
        let resolution = parse_group_mentions("@nobody @builder @all", &roster());
        assert!(resolution.saw_mention);
        assert!(resolution.mentions_all);
        assert_eq!(resolution.unresolved, vec!["nobody".to_string()]);
        assert_eq!(
            resolution.explicit_worker_ids,
            vec!["w-builder".to_string()]
        );
        // Invalid mentions are not hidden even when @all is also present.
        assert_eq!(
            resolution.resolve_targets(&roster()).unwrap_err(),
            MentionResolutionError::Unresolved {
                mentions: vec!["nobody".into()],
            }
        );
    }

    #[test]
    fn empty_roster_resolves_to_no_targets() {
        let resolution = parse_group_mentions("@anyone", &[]);
        assert!(resolution.saw_mention);
        assert_eq!(
            resolution.resolve_targets(&[]).unwrap_err(),
            MentionResolutionError::Unresolved {
                mentions: vec!["anyone".into()],
            }
        );
    }
}
