//! Semantic search over indexed symbols

use anyhow::Result;
use rusqlite::{params, Connection};

use super::embeddings::EmbeddingEngine;
use super::parser::SymbolType;

// Minimal stop words — only articles, prepositions, and pronouns.
// Code-relevant words (get, set, new, type, self, etc.) are kept since
// they appear in symbol names and signatures.
const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "to", "of",
    "in", "for", "on", "with", "at", "by", "from", "as", "into", "about", "between", "through",
    "during", "before", "after", "above", "below", "and", "but", "or", "nor", "not", "so", "yet",
    "because", "if", "when", "where", "how", "what", "which", "who", "whom", "me", "my", "we",
    "our", "you", "your", "he", "him", "his", "she", "her", "they", "them", "their",
];

fn extract_search_words(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 2 && !STOP_WORDS.contains(&w.as_str()))
        .collect()
}

/// A search query
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Text query for semantic search
    pub text: Option<String>,
    /// Filter by symbol type
    pub symbol_type: Option<SymbolType>,
    /// Filter by file path pattern
    pub file_pattern: Option<String>,
    /// Maximum results to return
    pub limit: usize,
}

impl SearchQuery {
    pub fn new() -> Self {
        Self {
            limit: 20,
            ..Default::default()
        }
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn symbol_type(mut self, st: SymbolType) -> Self {
        self.symbol_type = Some(st);
        self
    }

    pub fn file_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.file_pattern = Some(pattern.into());
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

/// A search result
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: i64,
    pub symbol_type: SymbolType,
    pub symbol_name: String,
    pub symbol_path: String,
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub signature: Option<String>,
    pub score: f32,
}

/// Semantic retrieval engine
pub struct SemanticRetrieval<'a> {
    conn: &'a Connection,
    embeddings: Option<&'a EmbeddingEngine>,
}

impl<'a> SemanticRetrieval<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self {
            conn,
            embeddings: None,
        }
    }

    pub fn with_embeddings(mut self, engine: &'a EmbeddingEngine) -> Self {
        self.embeddings = Some(engine);
        self
    }

    /// Search for symbols matching the query
    pub async fn search(&self, codebase_id: &str, query: SearchQuery) -> Result<Vec<SearchResult>> {
        // If we have a text query and embeddings, do semantic search
        if let (Some(text), Some(engine)) = (&query.text, self.embeddings) {
            return self
                .semantic_search(codebase_id, text, &query, engine)
                .await;
        }

        // Otherwise do keyword/filter search
        self.keyword_search(codebase_id, &query)
    }

    /// Semantic search using embeddings
    async fn semantic_search(
        &self,
        codebase_id: &str,
        text: &str,
        query: &SearchQuery,
        engine: &EmbeddingEngine,
    ) -> Result<Vec<SearchResult>> {
        // Generate query embedding
        let query_embedding = engine.embed(text).await?;

        // Load candidate embeddings from database
        let candidates = self.load_candidates(codebase_id, query)?;

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // Calculate similarities
        let candidate_embeddings: Vec<(usize, Vec<f32>)> = candidates
            .iter()
            .enumerate()
            .filter_map(|(idx, (_, emb_opt, _))| emb_opt.clone().map(|e| (idx, e)))
            .collect();

        let scored =
            EmbeddingEngine::top_k_similar(&query_embedding, &candidate_embeddings, query.limit);

        // Build results
        let mut results = Vec::new();
        for (idx, score) in scored {
            if let Some((id, _, meta)) = candidates.get(idx) {
                results.push(SearchResult {
                    id: *id,
                    symbol_type: meta.symbol_type,
                    symbol_name: meta.symbol_name.clone(),
                    symbol_path: meta.symbol_path.clone(),
                    file_path: meta.file_path.clone(),
                    line_start: meta.line_start,
                    line_end: meta.line_end,
                    signature: meta.signature.clone(),
                    score,
                });
            }
        }

        Ok(results)
    }

    /// Load candidate symbols with embeddings (chunked for scalability)
    fn load_candidates(
        &self,
        codebase_id: &str,
        query: &SearchQuery,
    ) -> Result<Vec<SearchCandidate>> {
        const CHUNK_SIZE: i64 = 500;
        let mut all_candidates = Vec::new();
        let mut offset = 0;

        loop {
            let mut sql = String::from(
                "SELECT id, symbol_type, symbol_name, symbol_path, file_path,
                        line_start, line_end, signature, embedding
                 FROM codebase_index WHERE codebase_id = ?1 AND embedding IS NOT NULL",
            );

            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> =
                vec![Box::new(codebase_id.to_string())];

            if let Some(st) = &query.symbol_type {
                sql.push_str(" AND symbol_type = ?");
                params_vec.push(Box::new(st.as_str().to_string()));
            }

            if let Some(pattern) = &query.file_pattern {
                sql.push_str(" AND file_path LIKE ?");
                params_vec.push(Box::new(format!("%{}%", pattern)));
            }

            // Add pagination for chunked loading
            sql.push_str(&format!(" LIMIT {} OFFSET {}", CHUNK_SIZE, offset));

            let mut stmt = self.conn.prepare(&sql)?;

            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params_vec.iter().map(|b| b.as_ref()).collect();

            let chunk: Vec<SearchCandidate> = stmt
                .query_map(params_refs.as_slice(), |row| {
                    let id: i64 = row.get(0)?;
                    let symbol_type_str: String = row.get(1)?;
                    let symbol_name: String = row.get(2)?;
                    let symbol_path: String = row.get(3)?;
                    let file_path: String = row.get(4)?;
                    let line_start: i64 = row.get(5)?;
                    let line_end: i64 = row.get(6)?;
                    let signature: Option<String> = row.get(7)?;
                    let embedding_blob: Option<Vec<u8>> = row.get(8)?;

                    // Parse embedding blob with validation
                    let embedding_opt = match embedding_blob {
                        Some(blob) => EmbeddingEngine::blob_to_embedding(&blob),
                        None => None,
                    };

                    let symbol_type =
                        SymbolType::parse(&symbol_type_str).unwrap_or(SymbolType::Function);

                    Ok((
                        id,
                        embedding_opt,
                        SymbolMeta {
                            symbol_type,
                            symbol_name,
                            symbol_path,
                            file_path,
                            line_start: line_start as usize,
                            line_end: line_end as usize,
                            signature,
                        },
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            if chunk.is_empty() {
                break; // No more candidates
            }

            all_candidates.extend(chunk);
            offset += CHUNK_SIZE;

            // Stop if we have enough candidates (2x requested limit for better results)
            if query.limit > 0 && all_candidates.len() >= query.limit * 2 {
                break;
            }
        }

        Ok(all_candidates)
    }

    /// Keyword search with per-word OR matching and relevance ranking
    fn keyword_search(&self, codebase_id: &str, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let words = query
            .text
            .as_ref()
            .map(|t| extract_search_words(t))
            .unwrap_or_default();

        // Build SQL with per-word OR clauses for better matching
        let mut sql = String::from(
            "SELECT id, symbol_type, symbol_name, symbol_path, file_path,
                    line_start, line_end, signature
             FROM codebase_index WHERE codebase_id = ?1",
        );

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(codebase_id.to_string())];

        if !words.is_empty() {
            let word_clauses: Vec<String> = words
                .iter()
                .map(|_| {
                    let idx = params_vec.len() + 1;
                    let clause = format!("(symbol_name LIKE ?{idx} OR symbol_path LIKE ?{idx} OR signature LIKE ?{idx})");
                    clause
                })
                .collect();

            // Collect patterns after building clauses to avoid double borrow
            let patterns: Vec<String> = words.iter().map(|w| format!("%{w}%")).collect();
            for pattern in patterns {
                params_vec.push(Box::new(pattern));
            }

            sql.push_str(&format!(" AND ({})", word_clauses.join(" OR ")));
        }

        if let Some(st) = &query.symbol_type {
            sql.push_str(" AND symbol_type = ?");
            params_vec.push(Box::new(st.as_str().to_string()));
        }

        if let Some(pattern) = &query.file_pattern {
            sql.push_str(" AND file_path LIKE ?");
            params_vec.push(Box::new(format!("%{pattern}%")));
        }

        // Fetch more than limit to allow re-ranking
        let fetch_limit = if words.len() > 1 {
            query.limit * 5
        } else {
            query.limit
        };
        sql.push_str(&format!(" LIMIT {fetch_limit}"));

        let mut stmt = self.conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();

        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            let id: i64 = row.get(0)?;
            let symbol_type_str: String = row.get(1)?;
            let symbol_name: String = row.get(2)?;
            let symbol_path: String = row.get(3)?;
            let file_path: String = row.get(4)?;
            let line_start: i64 = row.get(5)?;
            let line_end: i64 = row.get(6)?;
            let signature: Option<String> = row.get(7)?;

            Ok((
                id,
                symbol_type_str,
                symbol_name,
                symbol_path,
                file_path,
                line_start,
                line_end,
                signature,
            ))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (
                id,
                symbol_type_str,
                symbol_name,
                symbol_path,
                file_path,
                line_start,
                line_end,
                signature,
            ) = row?;

            let symbol_type = SymbolType::parse(&symbol_type_str).unwrap_or(SymbolType::Function);

            // Score by how many query words match (name, path, or signature)
            let score = if words.is_empty() {
                1.0
            } else {
                let name_lower = symbol_name.to_lowercase();
                let path_lower = symbol_path.to_lowercase();
                let sig_lower = signature
                    .as_deref()
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                let matches = words
                    .iter()
                    .filter(|w| {
                        name_lower.contains(w.as_str())
                            || path_lower.contains(w.as_str())
                            || sig_lower.contains(w.as_str())
                    })
                    .count();
                matches as f32 / words.len() as f32
            };

            results.push(SearchResult {
                id,
                symbol_type,
                symbol_name,
                symbol_path,
                file_path,
                line_start: line_start as usize,
                line_end: line_end as usize,
                signature,
                score,
            });
        }

        // Sort by match count descending, then by name
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.symbol_name.cmp(&b.symbol_name))
        });
        results.truncate(query.limit);

        Ok(results)
    }

    /// Get a specific symbol by ID
    pub fn get_symbol(&self, symbol_id: i64) -> Result<Option<SearchResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, symbol_type, symbol_name, symbol_path, file_path,
                    line_start, line_end, signature
             FROM codebase_index WHERE id = ?1",
        )?;

        let result = stmt.query_row([symbol_id], |row| {
            let id: i64 = row.get(0)?;
            let symbol_type_str: String = row.get(1)?;
            let symbol_name: String = row.get(2)?;
            let symbol_path: String = row.get(3)?;
            let file_path: String = row.get(4)?;
            let line_start: i64 = row.get(5)?;
            let line_end: i64 = row.get(6)?;
            let signature: Option<String> = row.get(7)?;

            Ok((
                id,
                symbol_type_str,
                symbol_name,
                symbol_path,
                file_path,
                line_start,
                line_end,
                signature,
            ))
        });

        match result {
            Ok((
                id,
                symbol_type_str,
                symbol_name,
                symbol_path,
                file_path,
                line_start,
                line_end,
                signature,
            )) => {
                let symbol_type =
                    SymbolType::parse(&symbol_type_str).unwrap_or(SymbolType::Function);

                Ok(Some(SearchResult {
                    id,
                    symbol_type,
                    symbol_name,
                    symbol_path,
                    file_path,
                    line_start: line_start as usize,
                    line_end: line_end as usize,
                    signature,
                    score: 1.0,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Find symbols that call a given symbol
    pub fn find_callers(&self, codebase_id: &str, symbol_name: &str) -> Result<Vec<SearchResult>> {
        let pattern = format!("%\"{}%", symbol_name);

        let mut stmt = self.conn.prepare(
            "SELECT id, symbol_type, symbol_name, symbol_path, file_path,
                    line_start, line_end, signature
             FROM codebase_index WHERE codebase_id = ?1 AND calls LIKE ?2",
        )?;

        let rows = stmt.query_map(params![codebase_id, pattern], |row| {
            let id: i64 = row.get(0)?;
            let symbol_type_str: String = row.get(1)?;
            let symbol_name: String = row.get(2)?;
            let symbol_path: String = row.get(3)?;
            let file_path: String = row.get(4)?;
            let line_start: i64 = row.get(5)?;
            let line_end: i64 = row.get(6)?;
            let signature: Option<String> = row.get(7)?;

            Ok((
                id,
                symbol_type_str,
                symbol_name,
                symbol_path,
                file_path,
                line_start,
                line_end,
                signature,
            ))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (
                id,
                symbol_type_str,
                symbol_name,
                symbol_path,
                file_path,
                line_start,
                line_end,
                signature,
            ) = row?;

            let symbol_type = SymbolType::parse(&symbol_type_str).unwrap_or(SymbolType::Function);

            results.push(SearchResult {
                id,
                symbol_type,
                symbol_name,
                symbol_path,
                file_path,
                line_start: line_start as usize,
                line_end: line_end as usize,
                signature,
                score: 1.0,
            });
        }

        Ok(results)
    }
}

type SearchCandidate = (i64, Option<Vec<f32>>, SymbolMeta);

struct SymbolMeta {
    symbol_type: SymbolType,
    symbol_name: String,
    symbol_path: String,
    file_path: String,
    line_start: usize,
    line_end: usize,
    signature: Option<String>,
}
