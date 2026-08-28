//! Project-tree navigation, fuzzy ranking, and composer completion.

use crate::tui_v2::services::ProjectEntry;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileQuery<'a> {
    pub start: usize,
    pub end: usize,
    pub query: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Completion {
    pub text: String,
    pub cursor_byte: usize,
    pub keep_open: bool,
}

pub fn active_query(input: &str, cursor_byte: usize) -> Option<FileQuery<'_>> {
    let cursor_byte = cursor_byte.min(input.len());
    if !input.is_char_boundary(cursor_byte) {
        return None;
    }
    let prefix = &input[..cursor_byte];
    let start = prefix.rfind('@')?;
    if start > 0
        && !prefix[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
    {
        return None;
    }
    let query = &prefix[start + 1..];
    if query.chars().any(char::is_whitespace) || query.contains(['[', ']']) {
        return None;
    }
    Some(FileQuery {
        start,
        end: cursor_byte,
        query,
    })
}

pub fn suggestions<'a>(
    entries: &'a [ProjectEntry],
    input: &str,
    cursor_byte: usize,
) -> Vec<&'a ProjectEntry> {
    let Some(query) = active_query(input, cursor_byte) else {
        return Vec::new();
    };
    let query = query.query.trim_start_matches("./").to_lowercase();
    let scoped = query.contains('/');
    let (parent, needle) = query.rsplit_once('/').unwrap_or(("", query.as_str()));

    let mut matches = entries
        .iter()
        .filter(|entry| {
            if scoped {
                entry.parent == parent
            } else if needle.is_empty() {
                entry.parent.is_empty()
            } else {
                true
            }
        })
        .filter_map(|entry| score(entry, needle).map(|score| (entry, score)))
        .collect::<Vec<_>>();
    matches.sort_by(|(left, left_score), (right, right_score)| {
        if needle.is_empty() {
            right
                .is_directory()
                .cmp(&left.is_directory())
                .then_with(|| left.path.cmp(&right.path))
        } else {
            right_score
                .cmp(left_score)
                .then_with(|| right.is_directory().cmp(&left.is_directory()))
                .then_with(|| left.path.len().cmp(&right.path.len()))
                .then_with(|| left.path.cmp(&right.path))
        }
    });
    matches.into_iter().map(|(entry, _)| entry).collect()
}

pub fn complete_active_query(
    input: &str,
    cursor_byte: usize,
    entry: &ProjectEntry,
) -> Option<Completion> {
    let query = active_query(input, cursor_byte)?;
    let mut completed = String::with_capacity(input.len().saturating_add(entry.path.len() + 3));
    completed.push_str(&input[..query.start]);
    if entry.is_directory() {
        completed.push('@');
        completed.push_str(&entry.path);
        completed.push('/');
    } else {
        completed.push('[');
        completed.push_str(&entry.path);
        completed.push_str("] ");
    }
    let cursor_byte = completed.len();
    completed.push_str(&input[query.end..]);
    Some(Completion {
        text: completed,
        cursor_byte,
        keep_open: entry.is_directory(),
    })
}

fn score(entry: &ProjectEntry, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(if entry.is_directory() { 20 } else { 1 });
    }
    if entry.search_name == query {
        return Some(500);
    }
    if entry
        .search_name
        .rsplit_once('.')
        .is_some_and(|(stem, _)| stem == query)
    {
        return Some(475);
    }
    if entry.search_name.starts_with(query) {
        return Some(400);
    }
    if entry.search_name.contains(query) {
        return Some(300);
    }
    if entry.search_path.contains(query) {
        return Some(200);
    }
    subsequence_score(&entry.search_name, query)
        .map(|score| score + 100)
        .or_else(|| subsequence_score(&entry.search_path, query))
}

fn subsequence_score(candidate: &str, query: &str) -> Option<i32> {
    let mut candidate = candidate.char_indices();
    let mut previous_end = None;
    let mut score = 0_i32;
    for expected in query.chars() {
        let (index, matched) = candidate.find(|(_, character)| *character == expected)?;
        score += previous_end.map_or(
            12,
            |previous_end| {
                if index == previous_end {
                    16
                } else {
                    8
                }
            },
        );
        previous_end = Some(index + matched.len_utf8());
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use crate::tui_v2::services::ProjectEntryKind;

    use super::*;

    fn entry(path: &str, kind: ProjectEntryKind) -> ProjectEntry {
        let name = path.rsplit('/').next().unwrap_or(path);
        let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
        ProjectEntry {
            path: path.to_owned(),
            name: name.to_owned(),
            parent: parent.to_owned(),
            kind,
            search_path: path.to_lowercase(),
            search_name: name.to_lowercase(),
        }
    }

    #[test]
    fn query_requires_a_token_boundary() {
        assert!(active_query("email@example.com", 17).is_none());
        assert_eq!(
            active_query("Review @src/ma", 14),
            Some(FileQuery {
                start: 7,
                end: 14,
                query: "src/ma",
            })
        );
    }

    #[test]
    fn root_and_nested_queries_form_a_navigable_tree() {
        let entries = [
            entry("docs", ProjectEntryKind::Directory),
            entry("src", ProjectEntryKind::Directory),
            entry("src/main.rs", ProjectEntryKind::File),
            entry("src/model.rs", ProjectEntryKind::File),
            entry("tests/main_test.rs", ProjectEntryKind::File),
        ];
        let root = suggestions(&entries, "@", 1);
        assert_eq!(
            root.iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            ["docs", "src"]
        );
        let nested = suggestions(&entries, "@src/m", 6);
        assert_eq!(
            nested
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            ["src/main.rs", "src/model.rs"]
        );
    }

    #[test]
    fn directories_continue_browsing_and_files_insert_a_reference() {
        let directory = entry("src", ProjectEntryKind::Directory);
        assert_eq!(
            complete_active_query("Review @s", 9, &directory),
            Some(Completion {
                text: "Review @src/".to_owned(),
                cursor_byte: 12,
                keep_open: true,
            })
        );
        let file = entry("src/main.rs", ProjectEntryKind::File);
        assert_eq!(
            complete_active_query("Review @src/ma", 14, &file),
            Some(Completion {
                text: "Review [src/main.rs] ".to_owned(),
                cursor_byte: 21,
                keep_open: false,
            })
        );
    }
}
