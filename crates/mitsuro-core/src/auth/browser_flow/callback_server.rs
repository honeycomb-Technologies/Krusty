use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};

/// Result from the OAuth callback
pub enum CallbackResult {
    Success { code: String },
    Error { error: String, description: String },
}

fn callback_timeout_result() -> CallbackResult {
    CallbackResult::Error {
        error: "callback_timeout".to_string(),
        description: "OAuth callback timed out with no browser response".to_string(),
    }
}

fn callback_receive_error(error: impl std::fmt::Display) -> CallbackResult {
    CallbackResult::Error {
        error: "server_error".to_string(),
        description: format!("Failed while waiting for OAuth callback: {}", error),
    }
}

fn receive_callback_request<T, E>(result: Result<Option<T>, E>) -> Result<T, CallbackResult>
where
    E: std::fmt::Display,
{
    match result {
        Ok(Some(request)) => Ok(request),
        Ok(None) => Err(callback_timeout_result()),
        Err(error) => Err(callback_receive_error(error)),
    }
}

fn parse_query_params(path: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Some(query) = path.split('?').nth(1) {
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            params.insert(key.to_string(), value.to_string());
        }
    }
    params
}

/// Run the local callback server
pub fn run_callback_server(port: u16, expected_state: String, tx: mpsc::Sender<CallbackResult>) {
    let addr = format!("127.0.0.1:{}", port);
    let server = match tiny_http::Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(CallbackResult::Error {
                error: "server_error".to_string(),
                description: format!("Failed to start callback server: {}", e),
            });
            return;
        }
    };

    let request = match receive_callback_request(server.recv_timeout(Duration::from_secs(300))) {
        Ok(request) => request,
        Err(error) => {
            let _ = tx.send(error);
            return;
        }
    };
    let path = request.url().to_string();
    let params = parse_query_params(&path);

    let state = params.get("state").map(|s| s.as_str()).unwrap_or("");
    if state != expected_state {
        let _ = tx.send(CallbackResult::Error {
            error: "state_mismatch".to_string(),
            description: "State parameter does not match".to_string(),
        });
        respond_with_error(request, "State mismatch - possible CSRF attack");
        return;
    }

    if let Some(error) = params.get("error") {
        let description = params
            .get("error_description")
            .map(|s| s.as_str())
            .unwrap_or("Unknown error");
        let _ = tx.send(CallbackResult::Error {
            error: error.clone(),
            description: description.to_string(),
        });
        respond_with_error(request, description);
        return;
    }

    if let Some(code) = params.get("code") {
        let _ = tx.send(CallbackResult::Success { code: code.clone() });
        respond_with_success(request);
    } else {
        let _ = tx.send(CallbackResult::Error {
            error: "missing_code".to_string(),
            description: "No authorization code received".to_string(),
        });
        respond_with_error(request, "No authorization code received");
    }
}

fn respond_with_success(request: tiny_http::Request) {
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>Authentication Successful</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
            background:
                radial-gradient(circle at top, rgba(117, 97, 126, 0.24), transparent 42%),
                #0c0d10;
            color: #e8e5ea;
        }
        .container {
            text-align: center;
            padding: 2rem;
            background: #19181d;
            border: 1px solid rgba(232, 229, 234, 0.12);
            box-shadow: 0 24px 70px rgba(0, 0, 0, 0.42);
            border-radius: 1rem;
            backdrop-filter: blur(10px);
        }
        h1 { font-size: 2rem; margin-bottom: 1rem; }
        p { opacity: 0.9; }
        .checkmark {
            font-size: 4rem;
            margin-bottom: 1rem;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="checkmark">✓</div>
        <h1>Authentication Successful!</h1>
        <p>You can close this window and return to Mitsuro.</p>
    </div>
</body>
</html>"#;

    let response = tiny_http::Response::from_string(html)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                .unwrap(),
        )
        .with_status_code(200);

    let _ = request.respond(response);
}

fn respond_with_error(request: tiny_http::Request, message: &str) {
    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Authentication Failed</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
            background:
                radial-gradient(circle at top, rgba(117, 97, 126, 0.18), transparent 42%),
                #0c0d10;
            color: #e8e5ea;
        }}
        .container {{
            text-align: center;
            padding: 2rem;
            background: #19181d;
            border: 1px solid rgba(180, 100, 108, 0.35);
            box-shadow: 0 24px 70px rgba(0, 0, 0, 0.42);
            border-radius: 1rem;
            backdrop-filter: blur(10px);
        }}
        h1 {{ font-size: 2rem; margin-bottom: 1rem; }}
        p {{ opacity: 0.9; }}
        .error-icon {{
            font-size: 4rem;
            margin-bottom: 1rem;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="error-icon">✗</div>
        <h1>Authentication Failed</h1>
        <p>{}</p>
        <p>Please close this window and try again.</p>
    </div>
</body>
</html>"#,
        html_escape(message)
    );

    let response = tiny_http::Response::from_string(html)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                .unwrap(),
        )
        .with_status_code(400);

    let _ = request.respond(response);
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Open a URL in the default browser
pub fn open_browser(url: &str) -> Result<()> {
    use std::process::Stdio;

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to open browser with xdg-open")?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to open browser with open")?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to open browser")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_query_params_extracts_values() {
        let params = parse_query_params("/callback?code=abc123&state=xyz789");
        assert_eq!(params.get("code"), Some(&"abc123".to_string()));
        assert_eq!(params.get("state"), Some(&"xyz789".to_string()));
    }

    #[test]
    fn receive_callback_request_reports_timeout() {
        let result = receive_callback_request::<(), std::io::Error>(Ok(None));

        match result {
            Err(CallbackResult::Error { error, description }) => {
                assert_eq!(error, "callback_timeout");
                assert!(description.contains("timed out"));
            }
            Ok(_) => panic!("timeout should not produce a request"),
            Err(CallbackResult::Success { .. }) => panic!("timeout should not succeed"),
        }
    }

    #[test]
    fn receive_callback_request_reports_server_error() {
        let result = receive_callback_request::<(), std::io::Error>(Err(std::io::Error::other(
            "socket failed",
        )));

        match result {
            Err(CallbackResult::Error { error, description }) => {
                assert_eq!(error, "server_error");
                assert!(description.contains("socket failed"));
            }
            Ok(_) => panic!("server error should not produce a request"),
            Err(CallbackResult::Success { .. }) => panic!("server error should not succeed"),
        }
    }
}
