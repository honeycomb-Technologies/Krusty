//! Offscreen MCP App renderer. On Linux this owns GTK/WebKitGTK on a dedicated
//! thread, so GPUI can stay on its native Wayland event loop while displaying
//! sandboxed app pixels inside the transcript.

use std::sync::mpsc;

use serde_json::Value;

const DEFAULT_WIDTH: u32 = 680;
const DEFAULT_HEIGHT: u32 = 420;

#[derive(Clone, Debug)]
pub enum McpAppRuntimeCommand {
    Load {
        key: String,
        html: String,
        width: u32,
        height: u32,
    },
    HostMessage {
        key: String,
        message: Value,
    },
    Click {
        key: String,
        x: f32,
        y: f32,
    },
    Key {
        key: String,
        value: String,
    },
    Capture {
        key: String,
    },
    Resize {
        key: String,
        width: u32,
        height: u32,
    },
    Close {
        key: String,
    },
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum McpAppRuntimeEvent {
    Started,
    Ready {
        key: String,
    },
    Frame {
        key: String,
        png: Vec<u8>,
        width: u32,
        height: u32,
    },
    HostMessage {
        key: String,
        message: Value,
    },
    FrameDirty {
        key: String,
    },
    OpenLink {
        key: String,
        url: String,
    },
    Error {
        key: Option<String>,
        message: String,
    },
}

pub struct McpAppRuntime {
    commands: mpsc::Sender<McpAppRuntimeCommand>,
    events: mpsc::Receiver<McpAppRuntimeEvent>,
}

impl McpAppRuntime {
    pub fn start() -> Result<Self, String> {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        platform::spawn(command_rx, event_tx)?;
        Ok(Self {
            commands: command_tx,
            events: event_rx,
        })
    }

    pub fn load(&self, key: String, html: String) -> Result<(), String> {
        self.send(McpAppRuntimeCommand::Load {
            key,
            html,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        })
    }

    pub fn send_host_message(&self, key: String, message: Value) -> Result<(), String> {
        self.send(McpAppRuntimeCommand::HostMessage { key, message })
    }

    pub fn click(&self, key: String, x: f32, y: f32) -> Result<(), String> {
        self.send(McpAppRuntimeCommand::Click { key, x, y })
    }

    pub fn key(&self, key: String, value: String) -> Result<(), String> {
        self.send(McpAppRuntimeCommand::Key { key, value })
    }

    pub fn capture(&self, key: String) -> Result<(), String> {
        self.send(McpAppRuntimeCommand::Capture { key })
    }

    pub fn resize(&self, key: String, width: u32, height: u32) -> Result<(), String> {
        self.send(McpAppRuntimeCommand::Resize { key, width, height })
    }

    pub fn close(&self, key: String) -> Result<(), String> {
        self.send(McpAppRuntimeCommand::Close { key })
    }

    pub fn try_recv(&self) -> Option<McpAppRuntimeEvent> {
        self.events.try_recv().ok()
    }

    fn send(&self, command: McpAppRuntimeCommand) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "MCP app renderer stopped".to_owned())
    }
}

impl Drop for McpAppRuntime {
    fn drop(&mut self) {
        let _ = self.commands.send(McpAppRuntimeCommand::Shutdown);
    }
}

#[cfg(all(target_os = "linux", feature = "mcp-app-runtime"))]
mod platform {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    use gtk::prelude::*;
    use webkit2gtk::{
        HardwareAccelerationPolicy, LoadEvent, PermissionRequestExt, SettingsExt, WebViewExt,
    };
    use wry::{NewWindowResponse, WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};

    use super::{McpAppRuntimeCommand, McpAppRuntimeEvent};

    const HOST_BRIDGE: &str = r#"
(() => {
  const nativePostMessage = window.postMessage.bind(window);
  const send = (kind, payload = {}) => {
    try { window.ipc.postMessage(JSON.stringify({ kind, ...payload })); } catch (_) {}
  };
  const encodeBlob = async blob => {
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = '';
    for (let offset = 0; offset < bytes.length; offset += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
    }
    return {
      base64: btoa(binary),
      mimeType: blob.type || 'application/octet-stream',
      size: bytes.length
    };
  };
  const sendHostMessage = message => {
    const blob = message?.method === 'ui/download-file' ? message?.params?.blob : null;
    if (!(blob instanceof Blob)) {
      send('host-message', { message });
      return;
    }
    encodeBlob(blob).then(encoded => {
      send('host-message', {
        message: { ...message, params: { ...message.params, blob: encoded } }
      });
    }).catch(error => {
      send('host-message', {
        message: { ...message, params: { ...message.params, blob: null },
          __mitsuroBlobError: String(error || 'Could not read download data') }
      });
    });
  };
  Object.defineProperty(window, '__mitsuroDeliver', {
    value: message => nativePostMessage(message, '*'), configurable: false
  });
  window.postMessage = (message, _targetOrigin, _transfer) => {
    sendHostMessage(message);
  };
  window.open = url => { send('open-link', { url: String(url || '') }); return null; };
  document.addEventListener('click', event => {
    const anchor = event.target && event.target.closest ? event.target.closest('a[href]') : null;
    if (!anchor) return;
    event.preventDefault();
    send('open-link', { url: anchor.href });
  }, true);
  let dirtyTimer = 0;
  const dirty = () => {
    clearTimeout(dirtyTimer);
    dirtyTimer = setTimeout(() => send('frame-dirty'), 24);
  };
  new MutationObserver(dirty).observe(document.documentElement, {
    subtree: true, childList: true, attributes: true, characterData: true
  });
  addEventListener('input', dirty, true);
  addEventListener('scroll', dirty, true);
  addEventListener('resize', dirty, true);
})();
"#;

    struct RuntimeView {
        window: gtk::OffscreenWindow,
        webview: wry::WebView,
        width: u32,
        height: u32,
    }

    pub fn spawn(
        commands: mpsc::Receiver<McpAppRuntimeCommand>,
        events: mpsc::Sender<McpAppRuntimeEvent>,
    ) -> Result<(), String> {
        std::thread::Builder::new()
            .name("mitsuro-mcp-app-webkit".to_owned())
            .spawn(move || run(commands, events))
            .map(|_| ())
            .map_err(|error| format!("could not start MCP app renderer: {error}"))
    }

    fn run(
        commands: mpsc::Receiver<McpAppRuntimeCommand>,
        events: mpsc::Sender<McpAppRuntimeEvent>,
    ) {
        if let Err(error) = gtk::init() {
            let _ = events.send(McpAppRuntimeEvent::Error {
                key: None,
                message: format!("WebKitGTK initialization failed: {error}"),
            });
            return;
        }
        let _ = events.send(McpAppRuntimeEvent::Started);
        let mut views = HashMap::<String, RuntimeView>::new();
        let mut running = true;
        while running {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            match commands.recv_timeout(Duration::from_millis(8)) {
                Ok(command) => {
                    running = handle_command(command, &mut views, &events);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        views.clear();
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
    }

    fn handle_command(
        command: McpAppRuntimeCommand,
        views: &mut HashMap<String, RuntimeView>,
        events: &mpsc::Sender<McpAppRuntimeEvent>,
    ) -> bool {
        match command {
            McpAppRuntimeCommand::Load {
                key,
                html,
                width,
                height,
            } => match build_view(key.clone(), html, width, height, events.clone()) {
                Ok(view) => {
                    views.insert(key, view);
                }
                Err(message) => {
                    let _ = events.send(McpAppRuntimeEvent::Error {
                        key: Some(key),
                        message,
                    });
                }
            },
            McpAppRuntimeCommand::HostMessage { key, message } => {
                if let Some(view) = views.get(&key) {
                    match serde_json::to_string(&message) {
                        Ok(message) => {
                            let script = format!("window.__mitsuroDeliver?.({message});");
                            if let Err(error) = view.webview.evaluate_script(&script) {
                                let _ = events.send(McpAppRuntimeEvent::Error {
                                    key: Some(key.clone()),
                                    message: format!("could not deliver host message: {error}"),
                                });
                            }
                            schedule_frame(
                                view.window.clone(),
                                key,
                                view.width,
                                view.height,
                                events.clone(),
                            );
                        }
                        Err(error) => {
                            let _ = events.send(McpAppRuntimeEvent::Error {
                                key: Some(key),
                                message: format!("could not encode host message: {error}"),
                            });
                        }
                    }
                }
            }
            McpAppRuntimeCommand::Click { key, x, y } => {
                if let Some(view) = views.get(&key) {
                    let script = format!(
                        "(() => {{ const e = document.elementFromPoint({x}, {y}); if (!e) return; e.focus?.(); e.dispatchEvent(new PointerEvent('pointerdown', {{bubbles:true,clientX:{x},clientY:{y},button:0}})); e.dispatchEvent(new MouseEvent('mousedown', {{bubbles:true,clientX:{x},clientY:{y},button:0}})); e.dispatchEvent(new PointerEvent('pointerup', {{bubbles:true,clientX:{x},clientY:{y},button:0}})); e.dispatchEvent(new MouseEvent('mouseup', {{bubbles:true,clientX:{x},clientY:{y},button:0}})); e.click?.(); }})()"
                    );
                    let _ = view.webview.evaluate_script(&script);
                    schedule_frame(
                        view.window.clone(),
                        key,
                        view.width,
                        view.height,
                        events.clone(),
                    );
                }
            }
            McpAppRuntimeCommand::Key { key, value } => {
                if let Some(view) = views.get(&key) {
                    let value = serde_json::to_string(&value).unwrap_or_else(|_| "\"\"".to_owned());
                    let script = format!(
                        "(() => {{ const el = document.activeElement; const key = {value}; if (!el) return; if (key === 'Backspace' && 'value' in el) {{ const s=el.selectionStart ?? el.value.length, e=el.selectionEnd ?? s; el.value=el.value.slice(0,Math.max(0,s-(s===e?1:0)))+el.value.slice(e); el.setSelectionRange?.(Math.max(0,s-1),Math.max(0,s-1)); el.dispatchEvent(new InputEvent('input',{{bubbles:true,inputType:'deleteContentBackward'}})); }} else if (key === 'Enter') {{ el.dispatchEvent(new KeyboardEvent('keydown',{{bubbles:true,key:'Enter'}})); if (el.tagName === 'TEXTAREA') document.execCommand('insertText',false,'\\n'); }} else if (key.length === 1) {{ document.execCommand('insertText',false,key); }} else {{ el.dispatchEvent(new KeyboardEvent('keydown',{{bubbles:true,key}})); }} }})()"
                    );
                    let _ = view.webview.evaluate_script(&script);
                    schedule_frame(
                        view.window.clone(),
                        key,
                        view.width,
                        view.height,
                        events.clone(),
                    );
                }
            }
            McpAppRuntimeCommand::Capture { key } => {
                if let Some(view) = views.get(&key) {
                    schedule_frame(
                        view.window.clone(),
                        key,
                        view.width,
                        view.height,
                        events.clone(),
                    );
                }
            }
            McpAppRuntimeCommand::Resize { key, width, height } => {
                if let Some(view) = views.get_mut(&key) {
                    view.width = width;
                    view.height = height;
                    view.window.set_default_size(width as i32, height as i32);
                    view.webview
                        .webview()
                        .set_size_request(width as i32, height as i32);
                    schedule_frame(
                        view.window.clone(),
                        key,
                        view.width,
                        view.height,
                        events.clone(),
                    );
                }
            }
            McpAppRuntimeCommand::Close { key } => {
                views.remove(&key);
            }
            McpAppRuntimeCommand::Shutdown => return false,
        }
        true
    }

    fn build_view(
        key: String,
        html: String,
        width: u32,
        height: u32,
        events: mpsc::Sender<McpAppRuntimeEvent>,
    ) -> Result<RuntimeView, String> {
        let window = gtk::OffscreenWindow::new();
        window.set_default_size(width as i32, height as i32);
        let ipc_key = key.clone();
        let ipc_events = events.clone();
        let nav_key = key.clone();
        let nav_events = events.clone();
        let initial_navigation = Arc::new(AtomicBool::new(true));
        let popup_key = key.clone();
        let popup_events = events.clone();
        let webview = WebViewBuilder::new()
            // Wry creates a WebKit ephemeral context for incognito views. MCP
            // app storage must never leak across transcript items or launches.
            .with_incognito(true)
            .with_initialization_script(HOST_BRIDGE)
            .with_clipboard(false)
            .with_ipc_handler(move |request| {
                handle_ipc(&ipc_key, request.body(), &ipc_events);
            })
            .with_navigation_handler(move |url| {
                if initial_navigation.swap(false, Ordering::AcqRel)
                    && (url == "about:blank" || url.starts_with("data:text/html"))
                {
                    true
                } else {
                    let _ = nav_events.send(McpAppRuntimeEvent::OpenLink {
                        key: nav_key.clone(),
                        url,
                    });
                    false
                }
            })
            .with_new_window_req_handler(move |url, _features| {
                let _ = popup_events.send(McpAppRuntimeEvent::OpenLink {
                    key: popup_key.clone(),
                    url,
                });
                NewWindowResponse::Deny
            })
            .build_gtk(&window)
            .map_err(|error| format!("could not create sandboxed WebKit view: {error}"))?;

        let raw = webview.webview();
        if !raw.is_ephemeral() {
            return Err("sandboxed WebKit view did not use an ephemeral context".to_owned());
        }
        raw.connect_permission_request(|_, request| {
            // No camera, microphone, geolocation, notification, device-info,
            // pointer-lock, DRM, or website-data permission is advertised.
            request.deny();
            true
        });
        if let Some(settings) = WebViewExt::settings(&raw) {
            settings.set_enable_developer_extras(false);
            settings.set_enable_html5_database(false);
            settings.set_enable_offline_web_application_cache(false);
            settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
        }
        let ready_window = window.clone();
        let ready_key = key.clone();
        let ready_events = events.clone();
        raw.connect_load_changed(move |_view, event| {
            if event == LoadEvent::Finished {
                let _ = ready_events.send(McpAppRuntimeEvent::Ready {
                    key: ready_key.clone(),
                });
                schedule_frame(
                    ready_window.clone(),
                    ready_key.clone(),
                    width,
                    height,
                    ready_events.clone(),
                );
            }
        });
        let crash_key = key;
        let crash_events = events;
        raw.connect_web_process_terminated(move |_, reason| {
            let _ = crash_events.send(McpAppRuntimeEvent::Error {
                key: Some(crash_key.clone()),
                message: format!("MCP app WebKit process terminated: {reason:?}"),
            });
        });
        window.show_all();
        raw.load_html(&html, None);
        Ok(RuntimeView {
            window,
            webview,
            width,
            height,
        })
    }

    fn handle_ipc(key: &str, body: &str, events: &mpsc::Sender<McpAppRuntimeEvent>) {
        let Ok(envelope) = serde_json::from_str::<serde_json::Value>(body) else {
            let _ = events.send(McpAppRuntimeEvent::Error {
                key: Some(key.to_owned()),
                message: "MCP app sent malformed bridge JSON".to_owned(),
            });
            return;
        };
        match envelope.get("kind").and_then(serde_json::Value::as_str) {
            Some("host-message") => {
                if let Some(message) = envelope.get("message").cloned() {
                    let _ = events.send(McpAppRuntimeEvent::HostMessage {
                        key: key.to_owned(),
                        message,
                    });
                }
            }
            Some("open-link") => {
                if let Some(url) = envelope.get("url").and_then(serde_json::Value::as_str) {
                    let _ = events.send(McpAppRuntimeEvent::OpenLink {
                        key: key.to_owned(),
                        url: url.to_owned(),
                    });
                }
            }
            Some("frame-dirty") => {
                let _ = events.send(McpAppRuntimeEvent::FrameDirty {
                    key: key.to_owned(),
                });
            }
            _ => {
                let _ = events.send(McpAppRuntimeEvent::Error {
                    key: Some(key.to_owned()),
                    message: "MCP app sent an unsupported bridge envelope".to_owned(),
                });
            }
        }
    }

    fn schedule_frame(
        window: gtk::OffscreenWindow,
        key: String,
        width: u32,
        height: u32,
        events: mpsc::Sender<McpAppRuntimeEvent>,
    ) {
        gtk::glib::timeout_add_local_once(Duration::from_millis(48), move || {
            let Some(pixbuf) = window.pixbuf() else {
                let _ = events.send(McpAppRuntimeEvent::Error {
                    key: Some(key),
                    message: "WebKit produced no offscreen frame".to_owned(),
                });
                return;
            };
            match pixbuf.save_to_bufferv("png", &[]) {
                Ok(png) => {
                    let _ = events.send(McpAppRuntimeEvent::Frame {
                        key,
                        png,
                        width,
                        height,
                    });
                }
                Err(error) => {
                    let _ = events.send(McpAppRuntimeEvent::Error {
                        key: Some(key),
                        message: format!("could not encode WebKit frame: {error}"),
                    });
                }
            }
        });
    }
}

#[cfg(all(test, target_os = "linux", feature = "mcp-app-runtime"))]
mod tests {
    use std::time::{Duration, Instant};

    use super::{McpAppRuntime, McpAppRuntimeEvent};

    #[test]
    fn live_webkit_probe_renders_and_bridges_json_rpc() {
        if std::env::var_os("MITSURO_RUN_MCP_APP_RUNTIME_TEST").is_none() {
            return;
        }
        let runtime = McpAppRuntime::start().expect("renderer thread starts");
        runtime
            .load(
                "probe".to_owned(),
                r#"<!doctype html><html><body style="margin:0;background:#112233;color:white"><button id="ready" style="position:absolute;left:0;top:0;width:80px;height:40px" onclick="window.parent.postMessage({jsonrpc:'2.0',method:'probe/click'},'*')">Ready</button><button id="download" style="position:absolute;left:100px;top:0;width:100px;height:40px" onclick="window.parent.postMessage({jsonrpc:'2.0',id:2,method:'ui/download-file',params:{name:'probe.txt',blob:new Blob(['hello'],{type:'text/plain'})}},'*')">Download</button><script>addEventListener('message',event=>{if(event.data?.id===1){document.querySelector('#ready').textContent='Initialized';window.parent.postMessage({jsonrpc:'2.0',method:'probe/host-response'},'*')}});window.parent.postMessage({jsonrpc:'2.0',id:1,method:'ui/initialize',params:{appInfo:{name:'probe',version:'1'}}},'*')</script></body></html>"#.to_owned(),
            )
            .expect("probe load command");
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut got_ready = false;
        let mut got_frame = false;
        let mut got_message = false;
        while Instant::now() < deadline && !(got_ready && got_frame && got_message) {
            match runtime.try_recv() {
                Some(McpAppRuntimeEvent::Ready { key }) if key == "probe" => got_ready = true,
                Some(McpAppRuntimeEvent::Frame { key, png, .. }) if key == "probe" => {
                    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
                    got_frame = true;
                }
                Some(McpAppRuntimeEvent::HostMessage { key, message }) if key == "probe" => {
                    assert_eq!(message["method"], "ui/initialize");
                    got_message = true;
                }
                Some(McpAppRuntimeEvent::Error { message, .. }) => panic!("{message}"),
                Some(_) | None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_ready, "WebKit view did not finish loading");
        assert!(got_frame, "WebKit view did not produce a PNG frame");
        assert!(got_message, "MCP app postMessage did not reach the host");

        runtime
            .send_host_message(
                "probe".to_owned(),
                serde_json::json!({"jsonrpc":"2.0","id":1,"result":{}}),
            )
            .expect("host response command");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got_host_response = false;
        while Instant::now() < deadline && !got_host_response {
            match runtime.try_recv() {
                Some(McpAppRuntimeEvent::HostMessage { key, message }) if key == "probe" => {
                    got_host_response = message["method"] == "probe/host-response";
                }
                Some(McpAppRuntimeEvent::Error { message, .. }) => panic!("{message}"),
                Some(_) | None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(
            got_host_response,
            "host JSON-RPC response did not reach the MCP app"
        );

        runtime
            .click("probe".to_owned(), 20.0, 10.0)
            .expect("click command");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got_click = false;
        while Instant::now() < deadline && !got_click {
            match runtime.try_recv() {
                Some(McpAppRuntimeEvent::HostMessage { key, message }) if key == "probe" => {
                    got_click = message["method"] == "probe/click";
                }
                Some(McpAppRuntimeEvent::Error { message, .. }) => panic!("{message}"),
                Some(_) | None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_click, "GPUI click forwarding did not reach the MCP app");

        runtime
            .click("probe".to_owned(), 120.0, 10.0)
            .expect("download click command");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got_download = false;
        while Instant::now() < deadline && !got_download {
            match runtime.try_recv() {
                Some(McpAppRuntimeEvent::HostMessage { key, message }) if key == "probe" => {
                    if message["method"] == "ui/download-file" {
                        assert_eq!(message["params"]["name"], "probe.txt");
                        assert_eq!(message["params"]["blob"]["base64"], "aGVsbG8=");
                        assert_eq!(message["params"]["blob"]["size"], 5);
                        got_download = true;
                    }
                }
                Some(McpAppRuntimeEvent::Error { message, .. }) => panic!("{message}"),
                Some(_) | None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_download, "MCP app Blob did not reach the host safely");

        runtime
            .resize("probe".to_owned(), 500, 300)
            .expect("resize command");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got_resized_frame = false;
        while Instant::now() < deadline && !got_resized_frame {
            match runtime.try_recv() {
                Some(McpAppRuntimeEvent::Frame {
                    key,
                    width,
                    height,
                    png,
                }) if key == "probe" && width == 500 && height == 300 => {
                    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
                    got_resized_frame = true;
                }
                Some(McpAppRuntimeEvent::Error { message, .. }) => panic!("{message}"),
                Some(_) | None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(
            got_resized_frame,
            "WebKit view did not resize and recapture"
        );
    }
}

#[cfg(not(all(target_os = "linux", feature = "mcp-app-runtime")))]
mod platform {
    use std::sync::mpsc;

    use super::{McpAppRuntimeCommand, McpAppRuntimeEvent};

    pub fn spawn(
        _commands: mpsc::Receiver<McpAppRuntimeCommand>,
        _events: mpsc::Sender<McpAppRuntimeEvent>,
    ) -> Result<(), String> {
        Err("this build does not include the MCP app runtime".to_owned())
    }
}
