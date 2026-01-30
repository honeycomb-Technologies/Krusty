//! Indexer - Orchestrates the codebase indexing process

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tracing::{info, warn};
use walkdir::WalkDir;

use super::codebase::{Codebase, CodebaseStore};
use super::embeddings::EmbeddingEngine;
use super::parser::{ParsedSymbol, RustParser, SymbolType};

/// Current index version (bump when format changes)
pub const INDEX_VERSION: i32 = 1;

/// Phase of the indexing process
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexPhase {
    Scanning,
    Parsing,
    Embedding,
    Storing,
    Complete,
}

impl IndexPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Scanning => "Scanning",
            Self::Parsing => "Parsing",
            Self::Embedding => "Embedding",
            Self::Storing => "Storing",
            Self::Complete => "Complete",
        }
    }
}

/// Progress update during indexing
#[derive(Debug, Clone)]
pub struct IndexProgress {
    pub phase: IndexPhase,
    pub current: usize,
    pub total: usize,
    pub current_file: Option<String>,
}

/// Orchestrates codebase indexing
pub struct Indexer {
    parser: RustParser,
    embeddings: Option<EmbeddingEngine>,
}

impl Indexer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            parser: RustParser::new()?,
            embeddings: None,
        })
    }

    /// Initialize with embeddings (lazy load)
    pub fn with_embeddings(mut self) -> Result<Self> {
        self.embeddings = Some(EmbeddingEngine::new()?);
        Ok(self)
    }

    /// Index a codebase synchronously (no embeddings)
    ///
    /// Use this when embeddings are disabled to avoid async runtime issues.
    pub fn index_codebase_sync(
        &mut self,
        conn: &Connection,
        path: &Path,
        progress_tx: Option<mpsc::UnboundedSender<IndexProgress>>,
    ) -> Result<Codebase> {
        if self.embeddings.is_some() {
            anyhow::bail!("index_codebase_sync cannot be used with embeddings enabled");
        }

        let send_progress = |progress: IndexProgress| {
            if let Some(ref tx) = progress_tx {
                let _ = tx.send(progress);
            }
        };

        let store = CodebaseStore::new(conn);
        let codebase = store.get_or_create(path)?;
        info!(codebase_id = %codebase.id, path = %codebase.path, "Starting sync index");

        send_progress(IndexProgress {
            phase: IndexPhase::Scanning,
            current: 0,
            total: 0,
            current_file: None,
        });

        let rust_files = self.scan_rust_files(path)?;
        let total_files = rust_files.len();
        info!(files = total_files, "Found Rust files to index");

        if total_files == 0 {
            send_progress(IndexProgress {
                phase: IndexPhase::Complete,
                current: 0,
                total: 0,
                current_file: None,
            });
            return Ok(codebase);
        }

        let mut all_symbols: Vec<(PathBuf, ParsedSymbol)> = Vec::new();
        for (idx, file_path) in rust_files.iter().enumerate() {
            send_progress(IndexProgress {
                phase: IndexPhase::Parsing,
                current: idx + 1,
                total: total_files,
                current_file: Some(file_path.display().to_string()),
            });

            match self.parse_file(file_path) {
                Ok(symbols) => {
                    for symbol in symbols {
                        all_symbols.push((file_path.clone(), symbol));
                    }
                }
                Err(e) => {
                    warn!(file = %file_path.display(), error = %e, "Failed to parse file");
                }
            }
        }

        let total_symbols = all_symbols.len();
        info!(symbols = total_symbols, "Extracted symbols");

        send_progress(IndexProgress {
            phase: IndexPhase::Storing,
            current: 0,
            total: total_symbols,
            current_file: None,
        });

        store.clear_index(&codebase.id)?;
        let now = Utc::now().to_rfc3339();

        for (idx, (file_path, symbol)) in all_symbols.into_iter().enumerate() {
            if idx % 100 == 0 {
                send_progress(IndexProgress {
                    phase: IndexPhase::Storing,
                    current: idx,
                    total: total_symbols,
                    current_file: None,
                });
            }
            self.insert_symbol(conn, &codebase.id, &file_path, &symbol, None, &now)?;
        }

        store.mark_indexed(&codebase.id, INDEX_VERSION)?;

        send_progress(IndexProgress {
            phase: IndexPhase::Complete,
            current: total_symbols,
            total: total_symbols,
            current_file: None,
        });

        info!(codebase_id = %codebase.id, symbols = total_symbols, "Sync indexing complete");
        store
            .get_by_id(&codebase.id)?
            .context("Codebase not found after indexing")
    }

    /// Index a codebase with progress reporting (async, supports embeddings)
    pub async fn index_codebase(
        &mut self,
        conn: &Connection,
        path: &Path,
        progress_tx: Option<mpsc::UnboundedSender<IndexProgress>>,
    ) -> Result<Codebase> {
        let send_progress = |progress: IndexProgress| {
            if let Some(ref tx) = progress_tx {
                let _ = tx.send(progress);
            }
        };

        // Get or create codebase entry
        let store = CodebaseStore::new(conn);
        let codebase = store.get_or_create(path)?;
        info!(codebase_id = %codebase.id, path = %codebase.path, "Starting index");

        // Phase 1: Scan for Rust files
        send_progress(IndexProgress {
            phase: IndexPhase::Scanning,
            current: 0,
            total: 0,
            current_file: None,
        });

        let rust_files = self.scan_rust_files(path)?;
        let total_files = rust_files.len();
        info!(files = total_files, "Found Rust files to index");

        if total_files == 0 {
            send_progress(IndexProgress {
                phase: IndexPhase::Complete,
                current: 0,
                total: 0,
                current_file: None,
            });
            return Ok(codebase);
        }

        // Phase 2: Parse files and extract symbols
        let mut all_symbols: Vec<(PathBuf, ParsedSymbol)> = Vec::new();

        for (idx, file_path) in rust_files.iter().enumerate() {
            send_progress(IndexProgress {
                phase: IndexPhase::Parsing,
                current: idx + 1,
                total: total_files,
                current_file: Some(file_path.display().to_string()),
            });

            match self.parse_file(file_path) {
                Ok(symbols) => {
                    for symbol in symbols {
                        all_symbols.push((file_path.clone(), symbol));
                    }
                }
                Err(e) => {
                    warn!(file = %file_path.display(), error = %e, "Failed to parse file");
                }
            }
        }

        let total_symbols = all_symbols.len();
        info!(symbols = total_symbols, "Extracted symbols");

        // Phase 3: Generate embeddings (optional, chunked for progress)
        const EMBED_CHUNK_SIZE: usize = 64;
        let embeddings: Vec<Option<Vec<f32>>> = if let Some(ref engine) = self.embeddings {
            send_progress(IndexProgress {
                phase: IndexPhase::Embedding,
                current: 0,
                total: total_symbols,
                current_file: None,
            });

            let texts: Vec<String> = all_symbols
                .iter()
                .map(|(_, sym)| self.symbol_to_embedding_text(sym))
                .collect();

            let mut all_embeddings: Vec<Option<Vec<f32>>> = Vec::with_capacity(texts.len());
            let mut failed = false;

            for chunk in texts.chunks(EMBED_CHUNK_SIZE) {
                if failed {
                    all_embeddings.extend(std::iter::repeat_with(|| None).take(chunk.len()));
                    continue;
                }
                match engine.embed_batch(chunk.to_vec()).await {
                    Ok(embs) => {
                        all_embeddings.extend(embs.into_iter().map(Some));
                        send_progress(IndexProgress {
                            phase: IndexPhase::Embedding,
                            current: all_embeddings.len(),
                            total: total_symbols,
                            current_file: None,
                        });
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to generate embeddings, continuing without");
                        all_embeddings.extend(std::iter::repeat_with(|| None).take(chunk.len()));
                        failed = true;
                    }
                }
            }

            all_embeddings
        } else {
            vec![None; total_symbols]
        };

        // Phase 4: Store in database
        send_progress(IndexProgress {
            phase: IndexPhase::Storing,
            current: 0,
            total: total_symbols,
            current_file: None,
        });

        // Clear existing index
        store.clear_index(&codebase.id)?;

        // Insert new symbols
        let now = Utc::now().to_rfc3339();
        for (idx, ((file_path, symbol), embedding)) in
            all_symbols.into_iter().zip(embeddings).enumerate()
        {
            if idx % 100 == 0 {
                send_progress(IndexProgress {
                    phase: IndexPhase::Storing,
                    current: idx,
                    total: total_symbols,
                    current_file: None,
                });
            }

            self.insert_symbol(
                conn,
                &codebase.id,
                &file_path,
                &symbol,
                embedding.as_deref(),
                &now,
            )?;
        }

        // Mark as indexed
        store.mark_indexed(&codebase.id, INDEX_VERSION)?;

        send_progress(IndexProgress {
            phase: IndexPhase::Complete,
            current: total_symbols,
            total: total_symbols,
            current_file: None,
        });

        info!(
            codebase_id = %codebase.id,
            symbols = total_symbols,
            "Indexing complete"
        );

        // Refresh codebase to get updated timestamp
        store
            .get_by_id(&codebase.id)?
            .context("Codebase not found after indexing")
    }

    /// Scan for Rust files in a directory
    fn scan_rust_files(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();

        for entry in WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_hidden(e) && !is_target_dir(e))
        {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "rs").unwrap_or(false) {
                files.push(path.to_path_buf());
            }
        }

        Ok(files)
    }

    /// Parse a single Rust file
    fn parse_file(&mut self, path: &Path) -> Result<Vec<ParsedSymbol>> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        self.parser.parse_file(path, &source)
    }

    /// Convert symbol to text for embedding
    fn symbol_to_embedding_text(&self, symbol: &ParsedSymbol) -> String {
        let mut text = format!("{} {}", symbol.symbol_type.as_str(), symbol.name);

        if let Some(ref sig) = symbol.signature {
            text.push_str(": ");
            text.push_str(sig);
        }

        if !symbol.calls.is_empty() {
            text.push_str(" calls: ");
            text.push_str(&symbol.calls.join(", "));
        }

        text
    }

    /// Insert a symbol into the database
    fn insert_symbol(
        &self,
        conn: &Connection,
        codebase_id: &str,
        file_path: &Path,
        symbol: &ParsedSymbol,
        embedding: Option<&[f32]>,
        indexed_at: &str,
    ) -> Result<()> {
        let file_path_str = file_path.to_string_lossy().to_string();
        let calls_json = serde_json::to_string(&symbol.calls)?;
        let embedding_blob = embedding.map(EmbeddingEngine::embedding_to_blob);

        conn.execute(
            "INSERT INTO codebase_index
             (codebase_id, symbol_type, symbol_name, symbol_path, file_path,
              line_start, line_end, signature, embedding, calls, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                codebase_id,
                symbol.symbol_type.as_str(),
                symbol.name,
                symbol.full_path,
                file_path_str,
                symbol.line_start as i64,
                symbol.line_end as i64,
                symbol.signature,
                embedding_blob,
                calls_json,
                indexed_at,
            ],
        )?;

        Ok(())
    }

    /// Get index statistics for a codebase
    pub fn get_stats(conn: &Connection, codebase_id: &str) -> Result<IndexStats> {
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM codebase_index WHERE codebase_id = ?1",
            [codebase_id],
            |row| row.get(0),
        )?;

        let by_type: Vec<(String, i64)> = {
            let mut stmt = conn.prepare(
                "SELECT symbol_type, COUNT(*) FROM codebase_index
                 WHERE codebase_id = ?1 GROUP BY symbol_type ORDER BY COUNT(*) DESC",
            )?;
            let rows = stmt.query_map([codebase_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        let files: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT file_path) FROM codebase_index WHERE codebase_id = ?1",
            [codebase_id],
            |row| row.get(0),
        )?;

        let with_embeddings: i64 = conn.query_row(
            "SELECT COUNT(*) FROM codebase_index WHERE codebase_id = ?1 AND embedding IS NOT NULL",
            [codebase_id],
            |row| row.get(0),
        )?;

        Ok(IndexStats {
            total_symbols: total as usize,
            symbols_by_type: by_type
                .into_iter()
                .filter_map(|(t, c)| SymbolType::parse(&t).map(|st| (st, c as usize)))
                .collect(),
            total_files: files as usize,
            symbols_with_embeddings: with_embeddings as usize,
        })
    }
}

impl Default for Indexer {
    fn default() -> Self {
        Self::new().expect("Failed to create default Indexer")
    }
}

/// Statistics about an index
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub total_symbols: usize,
    pub symbols_by_type: Vec<(SymbolType, usize)>,
    pub total_files: usize,
    pub symbols_with_embeddings: usize,
}

/// Check if entry is a hidden file/directory
fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| s.starts_with('.'))
        .unwrap_or(false)
}

/// Check if entry is the target directory
fn is_target_dir(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_dir()
        && entry
            .file_name()
            .to_str()
            .map(|s| s == "target" || s == "node_modules" || s == ".git")
            .unwrap_or(false)
}
