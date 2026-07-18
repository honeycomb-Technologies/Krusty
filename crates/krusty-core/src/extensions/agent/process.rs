use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::extensions::bun_runtime::BunRuntime;

use super::manifest::is_safe_env_name;
use super::{AgentExtensionManifest, ExtensionCallContext};

const HOST_PROTOCOL_VERSION: u32 = 1;
const HOST_FILE_NAME: &str = "agent-extension-host-v1.mjs";
const MAX_PROTOCOL_LINE_BYTES: usize = 4 * 1024 * 1024;
const WORKER_REAP_TIMEOUT: Duration = Duration::from_secs(2);

/// Owns the extension worker's process tree independently of `Child`.
///
/// Tokio's `kill_on_drop` only targets the direct child. Extensions can spawn
/// descendants, so Unix workers are placed in their own process group and the
/// group leader PID is retained even after the direct child exits. On Windows,
/// `taskkill /T` is the best available equivalent without requiring a Job
/// Object handle to live in the async process abstraction.
struct WorkerProcessTree {
    leader_pid: Option<u32>,
    armed: bool,
}

impl WorkerProcessTree {
    fn new(leader_pid: Option<u32>) -> Self {
        Self {
            leader_pid,
            armed: leader_pid.is_some(),
        }
    }

    fn force_kill_sync(&self) {
        if !self.armed {
            return;
        }
        let Some(pid) = self.leader_pid else {
            return;
        };

        #[cfg(unix)]
        if let Err(error) =
            crate::process::signals::signal_process_group(pid, libc::SIGKILL, "SIGKILL")
        {
            tracing::debug!(pid, %error, "Failed to kill agent-extension process group");
        }

        #[cfg(windows)]
        {
            // Drop cannot await. Launching taskkill still gives descendants a
            // tree-aware cleanup path while `Child::start_kill` handles the
            // worker itself below.
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }

        #[cfg(not(any(unix, windows)))]
        let _ = pid;
    }

    #[cfg(unix)]
    fn disarm_if_gone(&mut self) {
        let Some(pid) = self.leader_pid else {
            self.armed = false;
            return;
        };
        if matches!(
            crate::process::signals::process_group_exists(pid),
            Ok(false)
        ) {
            self.armed = false;
        }
    }

    #[cfg(not(unix))]
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for WorkerProcessTree {
    fn drop(&mut self) {
        self.force_kill_sync();
    }
}

async fn terminate_worker(child: &mut Child, process_tree: &mut WorkerProcessTree) {
    #[cfg(unix)]
    process_tree.force_kill_sync();

    #[cfg(windows)]
    let tree_killed = if let Some(pid) = process_tree.leader_pid {
        matches!(
            tokio::time::timeout(
                WORKER_REAP_TIMEOUT,
                Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status(),
            )
            .await,
            Ok(Ok(status)) if status.success()
        )
    } else {
        true
    };

    let _ = child.start_kill();
    let _ = tokio::time::timeout(WORKER_REAP_TIMEOUT, child.wait()).await;

    #[cfg(unix)]
    process_tree.disarm_if_gone();

    #[cfg(windows)]
    if tree_killed {
        process_tree.disarm();
    }

    #[cfg(not(any(unix, windows)))]
    process_tree.disarm();
}

/// Read one JSONL protocol record without allowing an extension to make the
/// host allocate an unbounded `String` before a newline arrives.
async fn read_protocol_line<R>(reader: &mut R, max_bytes: usize) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::with_capacity(max_bytes.min(8 * 1024));
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(line))
            };
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if consumed > max_bytes.saturating_sub(line.len()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("agent extension protocol line exceeds {max_bytes} bytes"),
            ));
        }

        line.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(line));
        }
    }
}

async fn reject_symlink(path: &Path, label: &str) -> Result<()> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "agent extension {label} cannot be a symbolic link: {}",
                path.display()
            )
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to inspect agent extension {label} {}",
                path.display()
            )
        }),
    }
}

async fn prepare_state_dir(
    runtime_dir: &Path,
    manifest: &AgentExtensionManifest,
) -> Result<PathBuf> {
    // Validate before the ID participates in a path, even when a caller
    // bypasses normal discovery and constructs a manifest directly.
    manifest.validate_id()?;

    tokio::fs::create_dir_all(runtime_dir)
        .await
        .with_context(|| {
            format!(
                "failed to create runtime directory {}",
                runtime_dir.display()
            )
        })?;
    let runtime_root = tokio::fs::canonicalize(runtime_dir)
        .await
        .with_context(|| {
            format!(
                "failed to canonicalize agent extension runtime directory {}",
                runtime_dir.display()
            )
        })?;
    if !tokio::fs::metadata(&runtime_root).await?.is_dir() {
        bail!(
            "agent extension runtime path is not a directory: {}",
            runtime_root.display()
        );
    }

    let state_root = runtime_root.join("state");
    reject_symlink(&state_root, "state root").await?;
    tokio::fs::create_dir_all(&state_root)
        .await
        .with_context(|| format!("failed to create state root {}", state_root.display()))?;
    let state_root = tokio::fs::canonicalize(&state_root)
        .await
        .with_context(|| format!("failed to canonicalize state root {}", state_root.display()))?;
    if state_root.parent() != Some(runtime_root.as_path()) {
        bail!("agent extension state root escapes its runtime directory");
    }

    let state_dir = state_root.join(&manifest.id);
    reject_symlink(&state_dir, "state directory").await?;
    tokio::fs::create_dir_all(&state_dir)
        .await
        .with_context(|| format!("failed to create state directory {}", state_dir.display()))?;
    let state_dir = tokio::fs::canonicalize(&state_dir).await.with_context(|| {
        format!(
            "failed to canonicalize agent extension '{}' state directory {}",
            manifest.id,
            state_dir.display()
        )
    })?;
    if state_dir.parent() != Some(state_root.as_path()) {
        bail!(
            "agent extension '{}' state directory escapes the runtime state root",
            manifest.id
        );
    }
    reject_symlink(&state_dir.join("state.json"), "state file").await?;

    Ok(state_dir)
}

/// Runtime registrations returned by an extension during initialization.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ReadyMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub protocol: u32,
    pub tools: Vec<RegisteredTool>,
    pub commands: Vec<RegisteredCommand>,
    pub events: Vec<String>,
    #[serde(default)]
    pub context_hook: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RegisteredTool {
    pub name: String,
    pub description: String,
    #[serde(default = "empty_object")]
    pub parameters: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RegisteredCommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

fn empty_object() -> Value {
    serde_json::json!({"type": "object", "properties": {}})
}

#[derive(Serialize)]
struct HostRequest<'a> {
    id: String,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event: Option<Value>,
    context: &'a ExtensionCallContext,
}

#[derive(Debug, Deserialize)]
struct HostResponse {
    id: String,
    ok: bool,
    #[serde(default)]
    result: Value,
    #[serde(default)]
    error: Option<String>,
}

pub(crate) struct AgentExtensionProcess {
    child: Child,
    process_tree: WorkerProcessTree,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    timeout: Duration,
    pub ready: ReadyMessage,
}

impl AgentExtensionProcess {
    pub async fn start(
        bun_runtime: &BunRuntime,
        runtime_dir: &Path,
        extension_dir: &Path,
        entry: &Path,
        manifest: &AgentExtensionManifest,
        working_dir: &Path,
    ) -> Result<Self> {
        manifest.validate_id()?;
        tokio::fs::create_dir_all(runtime_dir)
            .await
            .with_context(|| format!("failed to create {}", runtime_dir.display()))?;
        let host_path = runtime_dir.join(HOST_FILE_NAME);
        write_host_if_needed(&host_path).await?;

        let state_dir = prepare_state_dir(runtime_dir, manifest).await?;
        let state_file = state_dir.join("state.json");
        let bun = bun_runtime.binary_path().await?;
        let mut command = Command::new(bun);
        command
            .arg("run")
            .arg(&host_path)
            .arg(entry)
            .arg(&state_file)
            .arg(working_dir)
            .current_dir(extension_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .env_clear()
            .env("KRUSTY_EXTENSION_ID", &manifest.id)
            .env("KRUSTY_EXTENSION_DIR", extension_dir)
            .env("KRUSTY_EXTENSION_STATE_DIR", &state_dir)
            .env("KRUSTY_WORKING_DIR", working_dir);

        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        if let Some(home) = std::env::var_os("HOME") {
            command.env("HOME", home);
        }
        for name in &manifest.permissions.env {
            if is_safe_env_name(name) {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
        }

        #[cfg(unix)]
        command.process_group(0);

        let mut child = command.spawn().with_context(|| {
            format!("failed to start agent extension '{}' with Bun", manifest.id)
        })?;
        let mut process_tree = WorkerProcessTree::new(child.id());
        let timeout = Duration::from_millis(manifest.timeout_ms);

        let startup = async {
            let stdin = child
                .stdin
                .take()
                .context("extension worker stdin unavailable")?;
            let stdout = child
                .stdout
                .take()
                .context("extension worker stdout unavailable")?;
            let mut stdout = BufReader::new(stdout);

            let line = tokio::time::timeout(
                timeout,
                read_protocol_line(&mut stdout, MAX_PROTOCOL_LINE_BYTES),
            )
            .await
            .with_context(|| format!("agent extension '{}' startup timed out", manifest.id))?
            .with_context(|| {
                format!(
                    "agent extension '{}' registration could not be read",
                    manifest.id
                )
            })?;
            let Some(line) = line else {
                let status = child.try_wait().ok().flatten();
                bail!(
                    "agent extension '{}' exited before registration{}",
                    manifest.id,
                    status
                        .map(|value| format!(" ({value})"))
                        .unwrap_or_default()
                );
            };
            let ready: ReadyMessage = serde_json::from_slice(&line).with_context(|| {
                format!(
                    "agent extension '{}' produced an invalid registration message",
                    manifest.id
                )
            })?;
            if ready.message_type != "ready" || ready.protocol != HOST_PROTOCOL_VERSION {
                bail!(
                    "agent extension '{}' uses unsupported host protocol {}",
                    manifest.id,
                    ready.protocol
                );
            }

            Ok::<_, anyhow::Error>((stdin, stdout, ready))
        }
        .await;

        let (stdin, stdout, ready) = match startup {
            Ok(startup) => startup,
            Err(error) => {
                terminate_worker(&mut child, &mut process_tree).await;
                return Err(error);
            }
        };

        Ok(Self {
            child,
            process_tree,
            stdin,
            stdout,
            timeout,
            ready,
        })
    }

    pub async fn call_tool(
        &mut self,
        name: &str,
        params: Value,
        context: &ExtensionCallContext,
    ) -> Result<Value> {
        self.call("tool", Some(name), Some(params), None, context)
            .await
    }

    pub async fn call_command(
        &mut self,
        name: &str,
        argument: &str,
        context: &ExtensionCallContext,
    ) -> Result<Value> {
        self.call(
            "command",
            Some(name),
            Some(Value::String(argument.to_string())),
            None,
            context,
        )
        .await
    }

    pub async fn call_event(
        &mut self,
        name: &str,
        event: Value,
        context: &ExtensionCallContext,
    ) -> Result<Value> {
        self.call("event", Some(name), None, Some(event), context)
            .await
    }

    pub async fn call_context(&mut self, context: &ExtensionCallContext) -> Result<Value> {
        self.call("context", None, None, None, context).await
    }

    async fn call(
        &mut self,
        kind: &str,
        name: Option<&str>,
        params: Option<Value>,
        event: Option<Value>,
        context: &ExtensionCallContext,
    ) -> Result<Value> {
        let child_status = match self.child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                terminate_worker(&mut self.child, &mut self.process_tree).await;
                return Err(error).context("failed to inspect agent extension worker");
            }
        };
        if let Some(status) = child_status {
            terminate_worker(&mut self.child, &mut self.process_tree).await;
            bail!("agent extension worker exited with {status}");
        }

        let id = uuid::Uuid::new_v4().to_string();
        let request = HostRequest {
            id: id.clone(),
            kind,
            name,
            params,
            event,
            context,
        };
        let mut payload = serde_json::to_vec(&request)?;
        payload.push(b'\n');

        let exchange = tokio::time::timeout(self.timeout, async {
            self.stdin.write_all(&payload).await?;
            self.stdin.flush().await?;
            read_protocol_line(&mut self.stdout, MAX_PROTOCOL_LINE_BYTES).await
        })
        .await;
        let line = match exchange {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => {
                terminate_worker(&mut self.child, &mut self.process_tree).await;
                bail!("agent extension worker closed its output stream");
            }
            Ok(Err(error)) => {
                terminate_worker(&mut self.child, &mut self.process_tree).await;
                return Err(anyhow!(error)).context("agent extension protocol exchange failed");
            }
            Err(_) => {
                terminate_worker(&mut self.child, &mut self.process_tree).await;
                return Err(anyhow!("agent extension {kind} request timed out"));
            }
        };
        let response: HostResponse = match serde_json::from_slice(&line) {
            Ok(response) => response,
            Err(error) => {
                terminate_worker(&mut self.child, &mut self.process_tree).await;
                return Err(error).context("agent extension returned malformed JSON");
            }
        };
        if response.id != id {
            let received = response.id;
            terminate_worker(&mut self.child, &mut self.process_tree).await;
            bail!(
                "agent extension protocol response id mismatch: expected {}, received {}",
                id,
                received
            );
        }
        if response.ok {
            Ok(response.result)
        } else {
            bail!(
                "{}",
                response
                    .error
                    .unwrap_or_else(|| "agent extension request failed".to_string())
            )
        }
    }
}

impl Drop for AgentExtensionProcess {
    fn drop(&mut self) {
        self.process_tree.force_kill_sync();
        let _ = self.child.start_kill();
    }
}

async fn write_host_if_needed(path: &Path) -> Result<()> {
    match tokio::fs::read_to_string(path).await {
        Ok(current) if current == HOST_SOURCE => Ok(()),
        Ok(_) | Err(_) => {
            let temporary = temporary_host_path(path);
            tokio::fs::write(&temporary, HOST_SOURCE).await?;
            tokio::fs::rename(&temporary, path).await?;
            Ok(())
        }
    }
}

fn temporary_host_path(path: &Path) -> PathBuf {
    path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()))
}

// The bridge deliberately owns stdout: extension console output is redirected
// to stderr so one accidental console.log cannot corrupt the host protocol.
const HOST_SOURCE: &str = r#"
import { createInterface } from "node:readline";
import { constants } from "node:fs";
import { open, mkdir } from "node:fs/promises";
import { dirname } from "node:path";
import { pathToFileURL } from "node:url";

const [entry, stateFile, workingDirectory] = process.argv.slice(2);
const MAX_PERSISTED_STATE_BYTES = 1024 * 1024;
const NO_FOLLOW = constants.O_NOFOLLOW ?? 0;
const STATE_READ_FLAGS = constants.O_RDONLY | NO_FOLLOW;
const STATE_WRITE_FLAGS = constants.O_WRONLY | constants.O_CREAT | constants.O_TRUNC | NO_FOLLOW;
const writeProtocol = (value) => process.stdout.write(JSON.stringify(value) + "\n");
const writeLog = (...values) => process.stderr.write(values.map((value) => typeof value === "string" ? value : JSON.stringify(value)).join(" ") + "\n");
console.log = writeLog;
console.info = writeLog;
console.warn = writeLog;
console.error = writeLog;

const tools = new Map();
const commands = new Map();
const events = new Map();
const contextHooks = [];
let persisted = {};
try {
  const stateHandle = await open(stateFile, STATE_READ_FLAGS);
  try {
    const stateBytes = Buffer.alloc(MAX_PERSISTED_STATE_BYTES + 1);
    let bytesRead = 0;
    while (bytesRead < stateBytes.length) {
      const chunk = await stateHandle.read(stateBytes, bytesRead, stateBytes.length - bytesRead, bytesRead);
      if (chunk.bytesRead === 0) break;
      bytesRead += chunk.bytesRead;
    }
    if (bytesRead > MAX_PERSISTED_STATE_BYTES) throw new Error(`persisted state exceeds ${MAX_PERSISTED_STATE_BYTES} bytes`);
    persisted = JSON.parse(stateBytes.subarray(0, bytesRead).toString("utf8"));
  } finally {
    await stateHandle.close();
  }
} catch (error) {
  if (error?.code !== "ENOENT") writeLog(`Ignoring invalid persisted state: ${error?.message || String(error)}`);
}
const saveState = async () => {
  const encoded = JSON.stringify(persisted, null, 2);
  if (Buffer.byteLength(encoded, "utf8") > MAX_PERSISTED_STATE_BYTES) {
    throw new Error(`persisted state exceeds ${MAX_PERSISTED_STATE_BYTES} bytes`);
  }
  await mkdir(dirname(stateFile), { recursive: true });
  const stateHandle = await open(stateFile, STATE_WRITE_FLAGS, 0o600);
  try {
    await stateHandle.writeFile(encoded, "utf8");
    await stateHandle.sync();
  } finally {
    await stateHandle.close();
  }
};

const schemaOf = (value) => {
  if (!value) return { type: "object", properties: {} };
  if (typeof value.toJSONSchema === "function") {
    try { return value.toJSONSchema(); } catch (_) {}
  }
  if (value._def && typeof value._def === "object") return { type: "object", additionalProperties: true };
  return value;
};
const registerTool = (definition) => {
  if (!definition || typeof definition.name !== "string") throw new Error("registerTool requires a name");
  const execute = definition.execute || definition.handler;
  if (typeof execute !== "function") throw new Error(`tool ${definition.name} requires execute`);
  tools.set(definition.name, {
    name: definition.name,
    description: definition.description || "Agent extension tool",
    parameters: schemaOf(definition.parameters || definition.inputSchema || definition.args),
    execute,
  });
};
const registerCommand = (name, definition) => {
  if (typeof name === "object") { definition = name; name = definition.name; }
  if (typeof definition === "function") definition = { handler: definition };
  const handler = definition?.handler || definition?.execute;
  if (typeof name !== "string" || typeof handler !== "function") throw new Error("registerCommand requires a name and handler");
  commands.set(name.replace(/^\//, ""), { name: name.replace(/^\//, ""), description: definition.description || "", handler });
};
const on = (name, handler) => {
  if (typeof handler !== "function") throw new Error(`event ${name} requires a handler`);
  const handlers = events.get(name) || [];
  handlers.push(handler);
  events.set(name, handlers);
};
const addContext = (handler) => {
  if (typeof handler !== "function") throw new Error("context hook requires a function");
  contextHooks.push(handler);
};
const api = {
  registerTool,
  tool: registerTool,
  registerCommand,
  command: registerCommand,
  on,
  addContext,
  context: addContext,
  directory: workingDirectory,
  worktree: workingDirectory,
  project: { directory: workingDirectory },
  state: {
    get: (key, fallback = undefined) => Object.prototype.hasOwnProperty.call(persisted, key) ? persisted[key] : fallback,
    set: async (key, value) => {
      const existed = Object.prototype.hasOwnProperty.call(persisted, key);
      const previous = persisted[key];
      persisted[key] = value;
      try {
        await saveState();
      } catch (error) {
        if (existed) persisted[key] = previous;
        else delete persisted[key];
        throw error;
      }
    },
    delete: async (key) => {
      const existed = Object.prototype.hasOwnProperty.call(persisted, key);
      const previous = persisted[key];
      delete persisted[key];
      try {
        await saveState();
      } catch (error) {
        if (existed) persisted[key] = previous;
        throw error;
      }
    },
  },
  log: { debug: writeLog, info: writeLog, warn: writeLog, error: writeLog },
};

const normalizeObjectPlugin = (plugin) => {
  for (const [name, definition] of Object.entries(plugin?.tools || plugin?.tool || {})) {
    registerTool({ name, ...definition });
  }
  for (const [name, definition] of Object.entries(plugin?.commands || plugin?.command || {})) {
    registerCommand(name, definition);
  }
  if (typeof plugin?.context === "function") addContext(plugin.context);
  for (const [name, handler] of Object.entries(plugin?.hooks || plugin?.events || {})) {
    if (typeof handler === "function") on(name, handler);
  }
  for (const [name, handler] of Object.entries(plugin || {})) {
    if (name.includes(".") && typeof handler === "function") on(name, handler);
  }
};

try {
  const module = await import(pathToFileURL(entry).href);
  const candidates = [];
  if (module.default) candidates.push(module.default);
  for (const [name, value] of Object.entries(module)) if (name !== "default" && typeof value === "function") candidates.push(value);
  if (candidates.length === 0 && typeof module === "object") normalizeObjectPlugin(module);
  for (const candidate of candidates) {
    if (typeof candidate === "function") {
      const returned = await candidate(api);
      if (returned && typeof returned === "object") normalizeObjectPlugin(returned);
    } else if (candidate && typeof candidate === "object") {
      normalizeObjectPlugin(candidate);
    }
  }
  writeProtocol({
    type: "ready",
    protocol: 1,
    tools: [...tools.values()].map(({ execute, ...definition }) => definition),
    commands: [...commands.values()].map(({ handler, ...definition }) => definition),
    events: [...events.keys()],
    context_hook: contextHooks.length > 0,
  });
} catch (error) {
  writeLog(error?.stack || String(error));
  process.exit(1);
}

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of lines) {
  if (!line.trim()) continue;
  let request;
  try {
    request = JSON.parse(line);
    let result = null;
    if (request.kind === "tool") {
      const tool = tools.get(request.name);
      if (!tool) throw new Error(`unknown extension tool: ${request.name}`);
      result = await tool.execute(request.params ?? {}, request.context ?? {});
    } else if (request.kind === "command") {
      const command = commands.get(request.name);
      if (!command) throw new Error(`unknown extension command: ${request.name}`);
      result = await command.handler(request.params ?? "", request.context ?? {});
    } else if (request.kind === "event") {
      const handlers = events.get(request.name) || [];
      result = [];
      for (const handler of handlers) {
        if (request.name === "tool.execute.before" || request.name === "tool.execute.after") {
          const input = request.event?.input ?? request.event ?? {};
          const output = structuredClone(request.event?.output ?? {});
          const returned = await handler(input, output, request.context ?? {});
          result.push(returned ?? output);
        } else {
          result.push(await handler(request.event, request.context ?? {}));
        }
      }
    } else if (request.kind === "context") {
      result = [];
      for (const handler of contextHooks) result.push(await handler(request.context ?? {}));
    } else {
      throw new Error(`unknown host request kind: ${request.kind}`);
    }
    writeProtocol({ id: request.id, ok: true, result: result ?? null });
  } catch (error) {
    writeProtocol({ id: request?.id || "unknown", ok: false, error: error?.stack || String(error) });
  }
}
"#;

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn environment_names_are_conservative() {
        assert!(is_safe_env_name("GITHUB_TOKEN"));
        assert!(!is_safe_env_name("GitHubToken"));
        assert!(!is_safe_env_name("BAD-NAME"));
    }

    #[test]
    fn host_source_declares_protocol_version() {
        assert!(HOST_SOURCE.contains("protocol: 1"));
        assert!(HOST_SOURCE.contains("registerTool"));
        assert!(HOST_SOURCE.contains("MAX_PERSISTED_STATE_BYTES"));
        assert!(HOST_SOURCE.contains("Buffer.alloc(MAX_PERSISTED_STATE_BYTES + 1)"));
        assert!(HOST_SOURCE.contains("O_NOFOLLOW"));
    }

    #[tokio::test]
    async fn protocol_reader_accepts_a_bounded_record() {
        let input = b"{\"ok\":true}\nremaining";
        let mut reader = BufReader::new(&input[..]);

        let line = read_protocol_line(&mut reader, 32)
            .await
            .expect("read protocol record")
            .expect("record should be present");

        assert_eq!(line, b"{\"ok\":true}\n");
        let remaining = read_protocol_line(&mut reader, 32)
            .await
            .expect("read trailing record")
            .expect("trailing record should be present");
        assert_eq!(remaining, b"remaining");
    }

    #[tokio::test]
    async fn protocol_reader_rejects_a_record_over_the_limit() {
        let input = vec![b'x'; 65];
        let mut reader = BufReader::new(&input[..]);

        let error = read_protocol_line(&mut reader, 64)
            .await
            .expect_err("oversized record must be rejected");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("exceeds 64 bytes"));
    }

    #[tokio::test]
    async fn extension_state_directory_is_a_direct_child_of_the_state_root() {
        let temp = TempDir::new().expect("temp dir");
        let runtime = temp.path().join("runtime");
        let manifest = AgentExtensionManifest {
            id: "contained-extension".to_string(),
            ..AgentExtensionManifest::default()
        };

        let state_dir = prepare_state_dir(&runtime, &manifest)
            .await
            .expect("prepare state directory");
        let state_root = runtime
            .join("state")
            .canonicalize()
            .expect("canonical state root");

        assert_eq!(state_dir.parent(), Some(state_root.as_path()));
        assert_eq!(
            state_dir.file_name(),
            Some(std::ffi::OsStr::new(manifest.id.as_str()))
        );
    }

    #[tokio::test]
    async fn invalid_extension_id_cannot_reach_a_parent_state_directory() {
        let temp = TempDir::new().expect("temp dir");
        let runtime = temp.path().join("runtime");
        let manifest = AgentExtensionManifest {
            id: "..".to_string(),
            ..AgentExtensionManifest::default()
        };

        assert!(prepare_state_dir(&runtime, &manifest).await.is_err());
        assert!(!runtime.join("state").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn extension_state_directory_rejects_a_symlink_escape() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().expect("temp dir");
        let runtime = temp.path().join("runtime");
        let state_root = runtime.join("state");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&state_root).expect("state root");
        fs::create_dir_all(&outside).expect("outside directory");
        symlink(&outside, state_root.join("escaped-extension")).expect("state symlink");
        let manifest = AgentExtensionManifest {
            id: "escaped-extension".to_string(),
            ..AgentExtensionManifest::default()
        };

        let error = prepare_state_dir(&runtime, &manifest)
            .await
            .expect_err("state symlink must be rejected");
        assert!(error.to_string().contains("symbolic link"));
        assert!(!outside.join("state.json").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn extension_state_file_rejects_a_symlink_escape() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().expect("temp dir");
        let runtime = temp.path().join("runtime");
        let state_dir = runtime.join("state").join("state-file-extension");
        let outside = temp.path().join("outside.json");
        fs::create_dir_all(&state_dir).expect("state directory");
        fs::write(&outside, "outside").expect("outside state target");
        symlink(&outside, state_dir.join("state.json")).expect("state file symlink");
        let manifest = AgentExtensionManifest {
            id: "state-file-extension".to_string(),
            ..AgentExtensionManifest::default()
        };

        let error = prepare_state_dir(&runtime, &manifest)
            .await
            .expect_err("state file symlink must be rejected");
        assert!(error.to_string().contains("symbolic link"));
        assert_eq!(
            fs::read_to_string(&outside).expect("outside state"),
            "outside"
        );
    }

    #[cfg(unix)]
    async fn spawn_test_process_tree() -> (Child, u32) {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("sleep 30 & child=$!; echo $child; wait")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        let mut child = command.spawn().expect("spawn process group");
        let pid = child.id().expect("worker pid");
        let stdout = child.stdout.take().expect("worker stdout");
        let mut stdout = BufReader::new(stdout);
        let descendant =
            tokio::time::timeout(Duration::from_secs(2), read_protocol_line(&mut stdout, 64))
                .await
                .expect("descendant announcement timed out")
                .expect("read descendant announcement")
                .expect("descendant announcement missing");
        let descendant = std::str::from_utf8(&descendant)
            .expect("descendant pid should be utf-8")
            .trim()
            .parse::<libc::pid_t>()
            .expect("descendant pid");
        assert_eq!(unsafe { libc::kill(descendant, 0) }, 0);
        (child, pid)
    }

    #[cfg(unix)]
    async fn assert_process_group_gone(pid: u32) {
        for _ in 0..50 {
            if !crate::process::signals::process_group_exists(pid)
                .expect("inspect terminated process group")
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !crate::process::signals::process_group_exists(pid)
                .expect("inspect terminated process group"),
            "worker process group survived termination"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn termination_kills_the_worker_process_group() {
        let (mut child, pid) = spawn_test_process_tree().await;
        let mut process_tree = WorkerProcessTree::new(Some(pid));

        assert!(
            crate::process::signals::process_group_exists(pid).expect("inspect live process group")
        );
        terminate_worker(&mut child, &mut process_tree).await;
        assert_process_group_gone(pid).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_the_guard_kills_the_worker_process_group() {
        let (mut child, pid) = spawn_test_process_tree().await;

        drop(WorkerProcessTree::new(Some(pid)));
        tokio::time::timeout(WORKER_REAP_TIMEOUT, child.wait())
            .await
            .expect("worker was not terminated")
            .expect("reap worker");
        assert_process_group_gone(pid).await;
    }
}
