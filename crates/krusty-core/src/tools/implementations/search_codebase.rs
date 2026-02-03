//! Search codebase tool - Query the semantic index for symbols

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::index::{CodebaseStore, EmbeddingEngine, SearchQuery, SemanticRetrieval, SymbolType};
use crate::storage::Database;
use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

pub struct SearchCodebaseTool {
    db_path: PathBuf,
    embedding_engine: Arc<RwLock<Option<Arc<EmbeddingEngine>>>>,
    codebase_path: String,
}

#[derive(Deserialize)]
struct Params {
    query: String,
    symbol_type: Option<String>,
    file_pattern: Option<String>,
    limit: Option<usize>,
}

impl SearchCodebaseTool {
    pub fn new(
        db_path: PathBuf,
        embedding_engine: Arc<RwLock<Option<Arc<EmbeddingEngine>>>>,
        codebase_path: String,
    ) -> Self {
        Self {
            db_path,
            embedding_engine,
            codebase_path,
        }
    }
}

#[async_trait]
impl Tool for SearchCodebaseTool {
    fn name(&self) -> &str {
        "search_codebase"
    }

    fn description(&self) -> &str {
        "Search the indexed codebase for symbols, functions, structs, and modules. Returns file paths, line ranges, and signatures. Use this BEFORE grep/glob for faster, smarter results."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query — symbol names, function names, descriptions"
                },
                "symbol_type": {
                    "type": "string",
                    "enum": ["function", "struct", "enum", "trait", "module", "impl", "const", "static", "type_alias", "macro"],
                    "description": "Filter results by symbol type"
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Filter results by file path substring (e.g. 'tools/' or 'streaming')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return (default: 15)",
                    "default": 15
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        // Clone what we need for the blocking task
        let db_path = self.db_path.clone();
        let codebase_path = self.codebase_path.clone();
        // Read the current engine from the shared ref (populated lazily by ensure_embedding_engine)
        let embedding_engine = self.embedding_engine.read().await.clone();

        // Run database operations on a blocking thread since rusqlite::Connection is !Send
        let result = tokio::task::spawn_blocking(move || {
            let db =
                Database::new(&db_path).map_err(|e| format!("Failed to open database: {}", e))?;
            let conn = db.conn();

            let codebase_id = match CodebaseStore::new(conn).get_by_path(&codebase_path) {
                Ok(Some(codebase)) => codebase.id,
                Ok(None) => {
                    return Err(
                        "Codebase not indexed. Run /index to build the codebase index first."
                            .to_string(),
                    );
                }
                Err(e) => {
                    return Err(format!("Failed to lookup codebase: {}", e));
                }
            };

            let mut search_query = SearchQuery::new()
                .text(&params.query)
                .limit(params.limit.unwrap_or(15));

            if let Some(ref st) = params.symbol_type {
                if let Some(symbol_type) = SymbolType::parse(st) {
                    search_query = search_query.symbol_type(symbol_type);
                }
            }

            if let Some(ref fp) = params.file_pattern {
                search_query = search_query.file_pattern(fp);
            }

            let mut retrieval = SemanticRetrieval::new(conn);
            if let Some(ref engine) = embedding_engine {
                retrieval = retrieval.with_embeddings(engine);
            }

            let results = futures::executor::block_on(retrieval.search(&codebase_id, search_query))
                .map_err(|e| format!("Search failed: {}", e))?;

            Ok(results)
        })
        .await;

        let results = match result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return ToolResult::error(e),
            Err(e) => return ToolResult::error(format!("Search task failed: {}", e)),
        };

        if results.is_empty() {
            return ToolResult::success("No matching symbols found.");
        }

        let json_results: Vec<Value> = results
            .iter()
            .map(|r| {
                json!({
                    "symbol_name": r.symbol_name,
                    "symbol_type": r.symbol_type.as_str(),
                    "file_path": r.file_path,
                    "line_start": r.line_start,
                    "line_end": r.line_end,
                    "signature": r.signature,
                    "score": format!("{:.2}", r.score),
                })
            })
            .collect();

        ToolResult::success(serde_json::to_string_pretty(&json_results).unwrap_or_default())
    }
}
