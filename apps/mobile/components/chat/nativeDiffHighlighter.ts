import { createHighlighterCore, type HighlighterCore, type ThemedToken } from "@shikijs/core";
import {
  KRUSTY_DIFF_THEME_NAMES,
  krustyDarkDiffTheme,
  krustyLightDiffTheme,
} from "./krustyDiffThemes";
import type { ToolDiffRow } from "./toolDiffModel";

export interface NativeDiffToken {
  content: string;
  color?: string;
}

let highlighterPromise: Promise<HighlighterCore | null> | undefined;
const tokenCache = new Map<string, NativeDiffToken[][]>();
const MAX_TOKEN_CACHE_ENTRIES = 12;

export async function highlightDiffRows(
  rows: ToolDiffRow[],
  filePath: string | undefined,
  scheme: "light" | "dark",
): Promise<NativeDiffToken[][] | null> {
  const highlighter = await getHighlighter();
  if (!highlighter) return null;

  const language = languageForPath(filePath);
  const theme = KRUSTY_DIFF_THEME_NAMES[scheme];
  const source = rows.map((row) => (row.kind === "metadata" ? "" : row.content)).join("\n");
  const sourceKey = `${source.length}:${source.slice(0, 96)}:${source.slice(-96)}`;
  const cacheKey = `${theme}\u0000${language}\u0000${sourceKey}`;
  const cached = tokenCache.get(cacheKey);
  if (cached) return cached;

  try {
    const highlighted = highlighter.codeToTokensBase(source, { lang: language, theme });
    const tokens = highlighted.map((line: ThemedToken[]) =>
      line.map((token) => ({ content: token.content, color: token.color })),
    );
    tokenCache.set(cacheKey, tokens);
    if (tokenCache.size > MAX_TOKEN_CACHE_ENTRIES) {
      tokenCache.delete(tokenCache.keys().next().value as string);
    }
    return tokens;
  } catch {
    return null;
  }
}

async function getHighlighter(): Promise<HighlighterCore | null> {
  if (!highlighterPromise) highlighterPromise = createNativeHighlighter();
  return highlighterPromise;
}

async function createNativeHighlighter(): Promise<HighlighterCore | null> {
  try {
    const [engine, ...languages] = await Promise.all([
      import("react-native-shiki-engine"),
      import("@shikijs/langs/typescript"),
      import("@shikijs/langs/tsx"),
      import("@shikijs/langs/javascript"),
      import("@shikijs/langs/jsx"),
      import("@shikijs/langs/rust"),
      import("@shikijs/langs/json"),
      import("@shikijs/langs/markdown"),
      import("@shikijs/langs/bash"),
      import("@shikijs/langs/yaml"),
      import("@shikijs/langs/toml"),
      import("@shikijs/langs/python"),
      import("@shikijs/langs/swift"),
      import("@shikijs/langs/kotlin"),
      import("@shikijs/langs/css"),
      import("@shikijs/langs/html"),
      import("@shikijs/langs/sql"),
      import("@shikijs/langs/go"),
      import("@shikijs/langs/c"),
      import("@shikijs/langs/cpp"),
      import("@shikijs/langs/csharp"),
      import("@shikijs/langs/java"),
      import("@shikijs/langs/ruby"),
      import("@shikijs/langs/php"),
    ]);
    if (!engine.isNativeEngineAvailable()) return null;
    return createHighlighterCore({
      themes: [krustyDarkDiffTheme, krustyLightDiffTheme],
      langs: languages.flatMap((language) => language.default),
      engine: engine.createNativeEngine({ maxCacheSize: 800 }),
    });
  } catch {
    return null;
  }
}

function languageForPath(filePath?: string): string {
  const name = (filePath ?? "").toLowerCase().split("/").pop() ?? "";
  if (name === "dockerfile") return "bash";
  const extension = name.includes(".") ? name.split(".").pop() ?? "" : "";
  return (
    {
      ts: "typescript",
      mts: "typescript",
      cts: "typescript",
      tsx: "tsx",
      js: "javascript",
      mjs: "javascript",
      cjs: "javascript",
      jsx: "jsx",
      rs: "rust",
      json: "json",
      md: "markdown",
      mdx: "markdown",
      sh: "bash",
      bash: "bash",
      zsh: "bash",
      yml: "yaml",
      yaml: "yaml",
      toml: "toml",
      py: "python",
      swift: "swift",
      kt: "kotlin",
      kts: "kotlin",
      css: "css",
      html: "html",
      htm: "html",
      sql: "sql",
      go: "go",
      c: "c",
      h: "c",
      cc: "cpp",
      cpp: "cpp",
      cxx: "cpp",
      hpp: "cpp",
      cs: "csharp",
      java: "java",
      rb: "ruby",
      php: "php",
    } as Record<string, string>
  )[extension] ?? "text";
}
