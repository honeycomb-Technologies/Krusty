use anyhow::{anyhow, bail, Context, Result};
use axum::http::HeaderMap;

pub fn resolve_public_base_url(headers: &HeaderMap) -> Result<String> {
    if let Ok(explicit) = std::env::var("KRUSTY_PUBLIC_BASE_URL") {
        return normalize_base_url(&explicit);
    }

    if let Some(forwarded) = headers.get("forwarded").and_then(header_value) {
        if let Some((proto, host)) = parse_forwarded(forwarded) {
            return compose_base_url(&proto, &host);
        }
    }

    let host = headers
        .get("x-forwarded-host")
        .and_then(header_value)
        .or_else(|| headers.get("host").and_then(header_value))
        .ok_or_else(|| anyhow!("Missing host header for public callback URL"))?;

    let proto = headers
        .get("x-forwarded-proto")
        .and_then(header_value)
        .unwrap_or("http");

    compose_base_url(proto, host)
}

fn normalize_base_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    let (proto, host) = split_scheme_host(trimmed)?;
    compose_base_url(proto, host)
}

fn compose_base_url(proto: &str, host: &str) -> Result<String> {
    let proto = proto
        .split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Missing protocol for public callback URL"))?;
    let host = host
        .split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Missing host for public callback URL"))?;

    if !matches!(proto, "http" | "https") {
        bail!("Unsupported callback URL protocol: {}", proto);
    }

    if host.contains('/') || host.contains('@') || host.chars().any(char::is_whitespace) {
        bail!("Invalid host for public callback URL");
    }

    Ok(format!("{}://{}", proto, host))
}

fn split_scheme_host(raw: &str) -> Result<(&str, &str)> {
    let (proto, host) = raw
        .split_once("://")
        .context("Public base URL must include scheme")?;

    if host.is_empty() {
        bail!("Public base URL missing host");
    }

    Ok((proto, host))
}

fn parse_forwarded(value: &str) -> Option<(String, String)> {
    let first = value.split(',').next()?.trim();
    let mut proto = None;
    let mut host = None;

    for segment in first.split(';') {
        let (key, raw_value) = segment.split_once('=')?;
        let value = raw_value.trim().trim_matches('"');
        match key.trim().to_ascii_lowercase().as_str() {
            "proto" => proto = Some(value.to_string()),
            "host" => host = Some(value.to_string()),
            _ => {}
        }
    }

    Some((proto?, host?))
}

fn header_value(value: &axum::http::HeaderValue) -> Option<&str> {
    value
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn prefers_forwarded_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "forwarded",
            HeaderValue::from_static(
                "for=1.2.3.4;proto=https;host=raspberrypi.example.ts.net:8443",
            ),
        );
        headers.insert("host", HeaderValue::from_static("127.0.0.1:3000"));

        assert_eq!(
            resolve_public_base_url(&headers).unwrap(),
            "https://raspberrypi.example.ts.net:8443"
        );
    }

    #[test]
    fn uses_forwarded_host_and_proto() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("raspberrypi.example.ts.net:8443"),
        );

        assert_eq!(
            resolve_public_base_url(&headers).unwrap(),
            "https://raspberrypi.example.ts.net:8443"
        );
    }

    #[test]
    fn falls_back_to_host_header() {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("127.0.0.1:3000"));

        assert_eq!(
            resolve_public_base_url(&headers).unwrap(),
            "http://127.0.0.1:3000"
        );
    }

    #[test]
    fn rejects_invalid_host_values() {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("bad host/path"));

        assert!(resolve_public_base_url(&headers).is_err());
    }
}
