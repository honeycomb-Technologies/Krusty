//! Server-side mention parsing for group rooms.
//!
//! Mentions route a group turn to selected members: `@slug` matches exactly,
//! `@Display Name` matches a Worker's spaced display name, `@all` (or no
//! mention at all) targets every member, and an ambiguous short prefix
//! selects every matching member (Grok-style) instead of failing the send.

/// One roster entry a mention can resolve to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMentionTarget {
    pub worker_id: String,
    pub slug: String,
    pub display_name: String,
}

/// Outcome of scanning one message against a roster.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MentionResolution {
    /// Whether the message contained any `@` mention token at all.
    pub saw_mention: bool,
    /// `@all` was present.
    pub mentions_all: bool,
    /// Explicitly resolved Worker ids in roster order, deduplicated.
    pub explicit_worker_ids: Vec<String>,
    /// Mention tokens that matched no roster entry.
    pub unresolved: Vec<String>,
}

impl MentionResolution {
    /// Effective turn targets: explicit selections when present, otherwise
    /// every member (`@all`, no mention, or only unresolved mentions).
    pub fn resolve_targets(&self, roster: &[GroupMentionTarget]) -> Vec<String> {
        if self.mentions_all || !self.saw_mention || self.explicit_worker_ids.is_empty() {
            return roster
                .iter()
                .map(|target| target.worker_id.clone())
                .collect();
        }
        self.explicit_worker_ids.clone()
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
        resolution.saw_mention = true;

        // Longest match wins so "@Deep Researcher" resolves to the spaced
        // display name instead of stopping at the token "Deep".
        if let Some((target, consumed)) = longest_display_name_match(after, roster) {
            matched_ids.push(target.worker_id.clone());
            index += 1 + consumed;
            continue;
        }

        let token = leading_mention_token(after);
        if token.is_empty() {
            index += 1;
            continue;
        }
        let token_len = token.len();
        let token = token.to_ascii_lowercase();
        if token == "all" {
            resolution.mentions_all = true;
        } else {
            let exact = roster
                .iter()
                .filter(|target| target.slug == token)
                .collect::<Vec<_>>();
            let candidates = if exact.is_empty() {
                // Ambiguous short prefix selects every matching member.
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
                matched_ids.extend(candidates.iter().map(|target| target.worker_id.clone()));
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
fn longest_display_name_match<'roster>(
    after: &str,
    roster: &'roster [GroupMentionTarget],
) -> Option<(&'roster GroupMentionTarget, usize)> {
    let mut best: Option<(&GroupMentionTarget, usize)> = None;
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
        // Only prefer the display name when it is longer than the plain slug
        // token would have been; a single-word name behaves like a token.
        if best.is_none_or(|(_, length)| name.len() > length) {
            best = Some((target, name.len()));
        }
    }
    // A display-name match shorter than or equal to the raw token is only
    // meaningful when it actually contains a space; otherwise the token path
    // (exact slug, then prefix) handles it with clearer semantics.
    best.filter(|(target, consumed)| {
        target.display_name.contains(' ') || *consumed > leading_mention_token(after).len()
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_group_mentions, GroupMentionTarget};

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

    fn target_ids(content: &str) -> Vec<String> {
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
            // Ambiguous short prefix selects all matching (Grok-style).
            (
                "@re check the diff",
                vec!["w-researcher".into(), "w-reviewer".into()],
            ),
            // Multiple mentions combine in roster order without duplicates.
            (
                "@builder and @researcher and @builder again",
                vec!["w-researcher".into(), "w-builder".into()],
            ),
            // Unresolved-only mentions fall back to everyone.
            ("@nobody around?", all.clone()),
            // Emails never mention.
            ("mail bob@researcher.dev instead", all),
            // Mid-sentence mention after punctuation resolves.
            ("cc: @reviewer, thanks", vec!["w-reviewer".into()]),
        ];
        for (content, expected) in cases {
            assert_eq!(target_ids(content), expected, "content: {content}");
        }
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
        // @all overrides explicit picks.
        assert_eq!(resolution.resolve_targets(&roster()).len(), 3);
    }

    #[test]
    fn empty_roster_resolves_to_no_targets() {
        let resolution = parse_group_mentions("@anyone", &[]);
        assert!(resolution.saw_mention);
        assert!(resolution.resolve_targets(&[]).is_empty());
    }
}
