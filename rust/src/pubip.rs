use std::net::IpAddr;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{Duration, Instant};

const SOURCES: &[&str] = &[
    "https://ipv4.icanhazip.com",
    "https://api.ipify.org",
    "https://ifconfig.me/ip",
    "https://ipinfo.io/ip",
    "https://myip.ipip.net",
];

pub fn extract_ipv4(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        if let IpAddr::V4(ipv4) = ip {
            return Some(ipv4.to_string());
        }
    }
    for token in
        trimmed.split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ':' || ch == '\u{ff1a}')
    {
        if let Ok(ip) = token.parse::<IpAddr>() {
            if let IpAddr::V4(ipv4) = ip {
                return Some(ipv4.to_string());
            }
        }
    }
    None
}

pub async fn discover() -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut last_error = None;
    for source in SOURCES {
        if Instant::now() >= deadline {
            break;
        }
        match fetch_source(source).await {
            Ok(body) => {
                if let Some(ip) = extract_ipv4(&body) {
                    return Ok(ip);
                }
                last_error = Some(anyhow::anyhow!("{source}: no IPv4 in response"));
            }
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no public IP sources available")))
}

async fn fetch_source(url: &str) -> Result<String> {
    let (host, path) = parse_https_url(url)?;
    let mut tls = crate::tlsutil::connect_unverified_tls(
        &format!("{host}:443"),
        &host,
        Duration::from_secs(3),
    )
    .await
    .with_context(|| format!("fetch public ip from {source}", source = url))?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: mars-rs/0.1\r\nConnection: close\r\n\r\n"
    );
    tls.write_all(request.as_bytes())
        .await
        .context("write public ip request")?;
    let mut response = Vec::new();
    tls.read_to_end(&mut response)
        .await
        .context("read public ip response")?;
    let text = String::from_utf8_lossy(&response);
    Ok(text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_else(|| text.to_string()))
}

fn parse_https_url(url: &str) -> Result<(String, String)> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| anyhow::anyhow!("only https sources are supported: {url}"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    Ok((host.to_string(), format!("/{path}")))
}
