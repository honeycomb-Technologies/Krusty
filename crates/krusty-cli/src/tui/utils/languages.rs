//! Language to file extension mapping
//!
//! Maps programming language names to their common file extensions.

/// Convert language name to file extensions
///
/// Used by extension LSP registration and language detection.
pub fn language_to_extensions(language: &str) -> Vec<String> {
    match language.to_lowercase().as_str() {
        "rust" => vec!["rs".into()],
        "python" => vec!["py".into(), "pyi".into()],
        "javascript" => vec!["js".into(), "mjs".into(), "cjs".into()],
        "typescript" => vec!["ts".into(), "mts".into(), "cts".into()],
        "typescriptreact" | "tsx" => vec!["tsx".into()],
        "javascriptreact" | "jsx" => vec!["jsx".into()],
        "go" => vec!["go".into()],
        "c" => vec!["c".into(), "h".into()],
        "cpp" | "c++" => vec!["cpp".into(), "hpp".into(), "cc".into(), "cxx".into()],
        "java" => vec!["java".into()],
        "ruby" => vec!["rb".into()],
        "lua" => vec!["lua".into()],
        "zig" => vec!["zig".into()],
        "toml" => vec!["toml".into()],
        "json" => vec!["json".into()],
        "yaml" => vec!["yaml".into(), "yml".into()],
        "markdown" => vec!["md".into()],
        "html" => vec!["html".into(), "htm".into()],
        "css" => vec!["css".into()],
        "scss" => vec!["scss".into()],
        "sass" => vec!["sass".into()],
        "vue" => vec!["vue".into()],
        "svelte" => vec!["svelte".into()],
        "elixir" => vec!["ex".into(), "exs".into()],
        "erlang" => vec!["erl".into()],
        "haskell" => vec!["hs".into()],
        "ocaml" => vec!["ml".into(), "mli".into()],
        "kotlin" => vec!["kt".into(), "kts".into()],
        "swift" => vec!["swift".into()],
        "scala" => vec!["scala".into()],
        "clojure" => vec!["clj".into(), "cljs".into(), "cljc".into()],
        "php" => vec!["php".into()],
        "r" => vec!["r".into(), "R".into()],
        "julia" => vec!["jl".into()],
        "dart" => vec!["dart".into()],
        "gleam" => vec!["gleam".into()],
        _ => vec![language.to_lowercase()],
    }
}
