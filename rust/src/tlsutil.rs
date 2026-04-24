use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_native_tls::TlsStream;

use crate::config::AgentConfig;

pub fn parse_fingerprint(value: &str) -> Result<[u8; 32]> {
    let mut normalized = value.trim().to_ascii_lowercase();
    if let Some(rest) = normalized.strip_prefix("sha256:") {
        normalized = rest.to_string();
    }
    let normalized = normalized.replace(':', "");
    let decoded = hex::decode(&normalized).context("decode fingerprint")?;
    if decoded.len() != 32 {
        anyhow::bail!("fingerprint must be 32 bytes, got {}", decoded.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&decoded);
    Ok(out)
}

pub async fn connect_pinned_tls(cfg: &AgentConfig) -> Result<TlsStream<TcpStream>> {
    let derived_host = relay_host(&cfg.relay);
    let server_name = cfg
        .server_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(derived_host.as_deref())
        .ok_or_else(|| anyhow::anyhow!("server_name is empty and relay host cannot be derived"))?
        .to_string();
    let want = parse_fingerprint(&cfg.fingerprint)?;
    let tcp = timeout(Duration::from_secs(10), TcpStream::connect(&cfg.relay))
        .await
        .context("tcp connect timeout")?
        .with_context(|| format!("dial {}", cfg.relay))?;

    let mut builder = tokio_native_tls::native_tls::TlsConnector::builder();
    builder
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .min_protocol_version(Some(tokio_native_tls::native_tls::Protocol::Tlsv12));
    let connector = tokio_native_tls::TlsConnector::from(builder.build().context("build tls")?);
    let tls = timeout(Duration::from_secs(10), connector.connect(&server_name, tcp))
        .await
        .context("tls handshake timeout")?
        .context("tls handshake")?;

    verify_peer_fingerprint(&tls, want)?;
    Ok(tls)
}

pub fn verify_peer_fingerprint(tls: &TlsStream<TcpStream>, want: [u8; 32]) -> Result<()> {
    let cert = tls
        .get_ref()
        .peer_certificate()
        .context("get peer certificate")?
        .ok_or_else(|| anyhow::anyhow!("server presented no certificate"))?;
    let der = cert.to_der().context("encode peer certificate der")?;
    let got = Sha256::digest(&der);
    if got.as_slice() != want {
        anyhow::bail!("certificate fingerprint mismatch: got sha256:{}", hex::encode(got));
    }
    Ok(())
}

pub fn relay_host(relay: &str) -> Option<String> {
    relay.rsplit_once(':').map(|(host, _)| host.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_go_fingerprint_format_case_insensitive_with_colons() {
        let fp = parse_fingerprint(
            "SHA256:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF",
        )
        .unwrap();

        assert_eq!(
            hex::encode(fp),
            "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"
        );
    }
}
