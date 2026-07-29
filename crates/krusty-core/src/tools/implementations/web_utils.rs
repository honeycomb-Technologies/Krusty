//! Shared helpers for local web tools.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures::StreamExt;
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{CONTENT_TYPE, LOCATION, USER_AGENT};
use reqwest::{Client, Response};
use serde_json::json;
use tokio::net::lookup_host;
use url::{Host, Url};

use crate::tools::ToolResult;

const DEFAULT_USER_AGENT: &str = "Mitsuro/1.0 (+https://github.com/honeycomb-Technologies/Krusty)";
const DEFAULT_TIMEOUT_SECS: u64 = 20;

static SCRIPT_STYLE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?is)<script\b[^>]*>.*?</script>|<style\b[^>]*>.*?</style>|<noscript\b[^>]*>.*?</noscript>|<svg\b[^>]*>.*?</svg>|<template\b[^>]*>.*?</template>",
    )
    .expect("script/style regex should compile")
});
static COMMENT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<!--.*?-->").expect("comment regex should compile"));
static TITLE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("title regex should compile")
});
static BREAK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)<\s*(br|/p|/div|/li|/h[1-6]|/tr|/section|/article)\b[^>]*>")
        .expect("break regex should compile")
});
static TAG_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<[^>]+>").expect("tag regex should compile"));
static SPACE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[ \t\r\x0c]+").expect("space regex should compile"));
static MANY_BLANKS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\n{3,}").expect("blank-line regex should compile"));

pub(super) fn web_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(DEFAULT_USER_AGENT)
        .build()
        .map_err(Into::into)
}

pub(super) fn validate_http_url(raw_url: &str) -> Result<Url> {
    let url = Url::parse(raw_url.trim()).map_err(|err| anyhow!("Invalid URL: {err}"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(anyhow!("URL userinfo is not supported."));
    }
    match url.scheme() {
        "http" | "https" => Ok(url),
        scheme => Err(anyhow!(
            "Unsupported URL scheme: {scheme}. Use http or https."
        )),
    }
}

async fn ensure_public_http_url(url: &Url) -> Result<()> {
    let Some(host) = url.host() else {
        return Err(anyhow!("URL must include a host."));
    };

    match host {
        Host::Ipv4(addr) => ensure_public_ip(IpAddr::V4(addr)),
        Host::Ipv6(addr) => ensure_public_ip(IpAddr::V6(addr)),
        Host::Domain(domain) => {
            let normalized = domain.trim_end_matches('.').to_ascii_lowercase();
            if normalized == "localhost" || normalized.ends_with(".localhost") {
                return Err(anyhow!("Refusing to fetch local host: {domain}"));
            }

            let port = url
                .port_or_known_default()
                .ok_or_else(|| anyhow!("URL must include a valid port."))?;
            let addrs = lookup_host((normalized.as_str(), port)).await?;
            let mut resolved = 0usize;
            for addr in addrs {
                resolved += 1;
                ensure_public_ip(addr.ip())?;
            }
            if resolved == 0 {
                return Err(anyhow!("Host did not resolve: {domain}"));
            }
            Ok(())
        }
    }
}

fn ensure_public_ip(ip: IpAddr) -> Result<()> {
    if is_blocked_ip(ip) {
        return Err(anyhow!(
            "Refusing to fetch private, loopback, link-local, or reserved address: {ip}"
        ));
    }
    Ok(())
}

fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(addr) => is_blocked_ipv4(addr),
        IpAddr::V6(addr) => is_blocked_ipv6(addr),
    }
}

fn is_blocked_ipv4(addr: Ipv4Addr) -> bool {
    let [a, b, _, _] = addr.octets();
    addr.is_private()
        || addr.is_loopback()
        || addr.is_link_local()
        || addr.is_broadcast()
        || addr.is_unspecified()
        || addr.is_documentation()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 198 && (18..=19).contains(&b))
        || a >= 224
}

fn is_blocked_ipv6(addr: Ipv6Addr) -> bool {
    let segments = addr.segments();
    addr.is_loopback()
        || addr.is_unspecified()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xff00) == 0xff00
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
}

pub(super) async fn get_limited(
    client: &Client,
    mut url: Url,
    max_bytes: usize,
) -> Result<(Url, String, Vec<u8>)> {
    let mut redirects = 0usize;
    loop {
        ensure_public_http_url(&url).await?;
        let response = client
            .get(url.clone())
            .header(USER_AGENT, DEFAULT_USER_AGENT)
            .send()
            .await?;

        if response.status().is_redirection() {
            if redirects >= 5 {
                return Err(anyhow!("Too many redirects while fetching URL"));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .ok_or_else(|| anyhow!("Redirect response missing Location header"))?
                .to_str()
                .map_err(|err| anyhow!("Invalid redirect Location header: {err}"))?;
            url = validate_http_url(url.join(location)?.as_str())?;
            redirects += 1;
            continue;
        }

        return read_limited_response(response, max_bytes).await;
    }
}

async fn read_limited_response(
    response: Response,
    max_bytes: usize,
) -> Result<(Url, String, Vec<u8>)> {
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("HTTP request failed with status {status}"));
    }

    if let Some(content_length) = response.content_length() {
        if content_length > max_bytes as u64 {
            return Err(anyhow!(
                "Response too large: {content_length} bytes exceeds limit of {max_bytes} bytes"
            ));
        }
    }

    let final_url = response.url().clone();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(anyhow!(
                "Response too large: exceeded limit of {max_bytes} bytes while downloading"
            ));
        }
        body.extend_from_slice(&chunk);
    }

    Ok((final_url, content_type, body))
}

pub(super) fn bytes_to_text(bytes: Vec<u8>, content_type: &str) -> Result<String> {
    if content_type
        .to_ascii_lowercase()
        .contains("charset=iso-8859-1")
    {
        return Ok(bytes.into_iter().map(char::from).collect());
    }

    String::from_utf8(bytes).map_err(|err| anyhow!("Response is not valid UTF-8 text: {err}"))
}

pub(super) fn html_title(html: &str) -> Option<String> {
    TITLE_RE
        .captures(html)
        .and_then(|caps| caps.get(1))
        .map(|value| normalize_whitespace(&decode_html_entities(value.as_str())))
        .filter(|title| !title.is_empty())
}

pub(super) fn html_to_text(html: &str) -> String {
    let without_scripts = SCRIPT_STYLE_RE.replace_all(html, " ");
    let without_comments = COMMENT_RE.replace_all(&without_scripts, " ");
    let with_breaks = BREAK_RE.replace_all(&without_comments, "\n");
    let without_tags = TAG_RE.replace_all(&with_breaks, " ");
    normalize_multiline_whitespace(&decode_html_entities(&without_tags))
}

pub(super) fn strip_tags_to_text(html_fragment: &str) -> String {
    let without_tags = TAG_RE.replace_all(html_fragment, " ");
    normalize_whitespace(&decode_html_entities(&without_tags))
}

pub(super) fn is_html(content_type: &str, url: &Url) -> bool {
    let lower = content_type.to_ascii_lowercase();
    lower.contains("text/html")
        || lower.contains("application/xhtml")
        || url.path().ends_with(".html")
        || url.path().ends_with(".htm")
}

pub(super) fn is_text_like(content_type: &str) -> bool {
    let lower = content_type.to_ascii_lowercase();
    lower.starts_with("text/")
        || lower.contains("json")
        || lower.contains("xml")
        || lower.contains("javascript")
        || lower.contains("x-www-form-urlencoded")
}

pub(super) fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_multiline_whitespace(text: &str) -> String {
    let normalized_lines = text
        .lines()
        .map(normalize_whitespace)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    MANY_BLANKS_RE
        .replace_all(&SPACE_RE.replace_all(&normalized_lines, " "), "\n\n")
        .trim()
        .to_string()
}

pub(super) fn decode_html_entities(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];

        let Some(end) = rest.find(';').filter(|end| *end <= 12) else {
            output.push('&');
            rest = &rest[1..];
            continue;
        };

        let entity = &rest[1..end];
        if let Some(decoded) = decode_entity(entity) {
            output.push(decoded);
            rest = &rest[end + 1..];
        } else {
            output.push('&');
            rest = &rest[1..];
        }
    }

    output.push_str(rest);
    output
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" => Some(' '),
        _ if entity.starts_with("#x") || entity.starts_with("#X") => {
            u32::from_str_radix(&entity[2..], 16)
                .ok()
                .and_then(char::from_u32)
        }
        _ if entity.starts_with('#') => entity[1..].parse::<u32>().ok().and_then(char::from_u32),
        _ => None,
    }
}

pub(super) fn tool_error(error: impl std::fmt::Display) -> ToolResult {
    ToolResult::error_with_details(
        "web_request_failed",
        error.to_string(),
        None,
        Some(json!({ "source": "local_web_tool" })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_to_text_strips_scripts_and_decodes_entities() {
        let text = html_to_text(
            "<html><head><title>T</title><script>bad()</script></head><body><h1>A &amp; B</h1><p>Hello&nbsp;world</p></body></html>",
        );

        assert!(text.contains("A & B"));
        assert!(text.contains("Hello world"));
        assert!(!text.contains("bad()"));
    }

    #[test]
    fn validate_http_url_rejects_non_http_schemes() {
        assert!(validate_http_url("https://example.com").is_ok());
        assert!(validate_http_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn blocked_ip_detects_private_and_loopback_ranges() {
        assert!(is_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("10.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("169.254.169.254".parse().unwrap()));
        assert!(is_blocked_ip("::1".parse().unwrap()));
        assert!(is_blocked_ip("fd00::1".parse().unwrap()));
        assert!(!is_blocked_ip("93.184.216.34".parse().unwrap()));
    }

    #[tokio::test]
    async fn ensure_public_http_url_rejects_localhost_without_dns() {
        let url = validate_http_url("http://localhost/test").unwrap();
        let err = ensure_public_http_url(&url).await.unwrap_err();

        assert!(err.to_string().contains("local host"));
    }

    #[tokio::test]
    async fn ensure_public_http_url_rejects_loopback_literal() {
        let url = validate_http_url("http://127.0.0.1/test").unwrap();
        let err = ensure_public_http_url(&url).await.unwrap_err();

        assert!(err.to_string().contains("Refusing to fetch"));
    }
}
