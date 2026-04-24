use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_native_tls::TlsStream;
use tokio_yamux::StreamHandle;

pub const MAX_FRAME_LEN: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentConfig {
    pub relay: String,
    pub server_name: Option<String>,
    pub fingerprint: String,
    pub token: String,
    pub agent_id: String,
    #[serde(default = "default_local_addr")]
    pub local_addr: String,
}

fn default_local_addr() -> String {
    "127.0.0.1:22".to_string()
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hello {
    #[serde(rename = "type")]
    pub kind: String,
    pub token: String,
    pub agent_id: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub hostname: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub os: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Response {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub assigned_port: u16,
    #[serde(default)]
    pub public_host: String,
}

pub fn parse_agent_config(yaml: &str) -> Result<AgentConfig> {
    let yaml = yaml.trim_start_matches('\u{feff}');
    let mut cfg: AgentConfig = serde_yaml::from_str(yaml).context("parse agent yaml")?;
    if cfg.local_addr.is_empty() {
        cfg.local_addr = default_local_addr();
    }
    if cfg.relay.is_empty() {
        anyhow::bail!("relay is required (host:port)");
    }
    if cfg.token.is_empty() {
        anyhow::bail!("token is required");
    }
    if cfg.agent_id.is_empty() {
        anyhow::bail!("agent_id is required");
    }
    if cfg.fingerprint.is_empty() {
        anyhow::bail!("fingerprint is empty");
    }
    if !cfg.fingerprint.starts_with("sha256:") {
        anyhow::bail!("fingerprint must start with sha256:");
    }
    parse_fingerprint(&cfg.fingerprint)?;
    Ok(cfg)
}

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

pub fn encode_json_frame<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let payload = serde_json::to_vec(value).context("marshal json frame")?;
    if payload.is_empty() || payload.len() > MAX_FRAME_LEN {
        anyhow::bail!("invalid frame length: {}", payload.len());
    }
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub async fn write_json_frame<W, T>(writer: &mut W, value: &T) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let frame = encode_json_frame(value)?;
    writer.write_all(&frame).await.context("write json frame")
}

pub async fn read_json_frame<R, T>(reader: &mut R) -> Result<T>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len = [0u8; 4];
    reader.read_exact(&mut len).await.context("read frame header")?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME_LEN {
        anyhow::bail!("invalid frame length: {}", len);
    }
    let mut payload = vec![0u8; len];
    reader
        .read_exact(&mut payload)
        .await
        .context("read frame payload")?;
    serde_json::from_slice(&payload).context("parse json frame")
}

pub fn load_agent_config(path: &str) -> Result<AgentConfig> {
    let content = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    parse_agent_config(&content)
}

pub async fn run_forever(cfg: AgentConfig) -> Result<()> {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);

    loop {
        match run_once(&cfg).await {
            Ok(()) => return Ok(()),
            Err(err) => {
                eprintln!("会话结束：{err:#}；{}s 后重连", backoff.as_secs());
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, max_backoff);
            }
        }
    }
}

pub async fn run_once(cfg: &AgentConfig) -> Result<()> {
    let tls = connect_pinned_tls(cfg).await?;
    let mut session = tokio_yamux::Session::new_client(tls, tokio_yamux::Config::default());
    let mut control = session.control();
    let (stream_tx, mut stream_rx) = tokio::sync::mpsc::channel::<StreamHandle>(32);

    let session_task = tokio::spawn(async move {
        while let Some(next) = session.next().await {
            let stream = next.context("yamux session")?;
            if stream_tx.send(stream).await.is_err() {
                break;
            }
        }
        Ok::<(), anyhow::Error>(())
    });

    let mut ctrl = control.open_stream().await.context("open control stream")?;
    let hostname = hostname::get()
        .ok()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_default();
    write_json_frame(
        &mut ctrl,
        &Hello {
            kind: "hello".to_string(),
            token: cfg.token.clone(),
            agent_id: cfg.agent_id.clone(),
            hostname,
            os: std::env::consts::OS.to_string(),
        },
    )
    .await
    .context("send hello")?;
    let response: Response = read_json_frame(&mut ctrl).await.context("read hello response")?;
    let _ = ctrl.shutdown().await;

    if response.kind != "ok" {
        control.close().await;
        anyhow::bail!("handshake rejected: {}", response.reason);
    }

    let public_host = if response.public_host.is_empty() {
        relay_host(&cfg.relay).unwrap_or_else(|| cfg.relay.clone())
    } else {
        response.public_host.clone()
    };
    eprintln!("=====================================================");
    eprintln!(" 已注册到中转 {}", cfg.relay);
    eprintln!(" AI 或用户现在可以这样连到本机：");
    eprintln!("     ssh -p {} user@{}", response.assigned_port, public_host);
    eprintln!(" 进来的流量会桥接到 {}", cfg.local_addr);
    eprintln!("=====================================================");

    while let Some(stream) = stream_rx.recv().await {
        let local_addr = cfg.local_addr.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_stream(local_addr, stream).await {
                eprintln!("stream bridge failed: {err:#}");
            }
        });
    }

    session_task.await.context("join yamux session")??;
    Ok(())
}

async fn connect_pinned_tls(cfg: &AgentConfig) -> Result<TlsStream<TcpStream>> {
    let derived_host = relay_host(&cfg.relay);
    let server_name = cfg
        .server_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(derived_host.as_deref())
        .ok_or_else(|| anyhow!("server_name is empty and relay host cannot be derived"))?
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

    let cert = tls
        .get_ref()
        .peer_certificate()
        .context("get peer certificate")?
        .ok_or_else(|| anyhow!("server presented no certificate"))?;
    let der = cert.to_der().context("encode peer certificate der")?;
    let got = Sha256::digest(&der);
    if got.as_slice() != want {
        anyhow::bail!("certificate fingerprint mismatch: got sha256:{}", hex::encode(got));
    }
    Ok(tls)
}

async fn handle_stream(mut local_addr: String, mut stream: StreamHandle) -> Result<()> {
    if local_addr.is_empty() {
        local_addr = default_local_addr();
    }
    let mut local = timeout(Duration::from_secs(5), TcpStream::connect(&local_addr))
        .await
        .context("local connect timeout")?
        .with_context(|| format!("dial {local_addr}"))?;
    let _ = tokio::io::copy_bidirectional(&mut local, &mut stream)
        .await
        .context("copy stream")?;
    Ok(())
}

fn relay_host(relay: &str) -> Option<String> {
    relay.rsplit_once(':').map(|(host, _)| host.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_go_agent_yaml_and_defaults_local_addr() {
        let cfg = parse_agent_config(
            "\u{feff}
relay: relay.example.com:7000
server_name: relay.example.com
fingerprint: sha256:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff
token: secret
agent_id: node-a
",
        )
        .unwrap();

        assert_eq!(cfg.relay, "relay.example.com:7000");
        assert_eq!(cfg.server_name.as_deref(), Some("relay.example.com"));
        assert_eq!(cfg.local_addr, "127.0.0.1:22");
    }

    #[test]
    fn parses_go_fingerprint_format_case_insensitive_with_colons() {
        let fp = parse_fingerprint(
            "SHA256:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF",
        )
        .unwrap();

        assert_eq!(hex::encode(fp), "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff");
    }

    #[test]
    fn encodes_go_compatible_length_prefixed_hello_json() {
        let hello = Hello {
            kind: "hello".to_string(),
            token: "secret".to_string(),
            agent_id: "node-a".to_string(),
            hostname: "host-a".to_string(),
            os: "windows".to_string(),
        };

        let frame = encode_json_frame(&hello).unwrap();
        let payload = br#"{"type":"hello","token":"secret","agent_id":"node-a","hostname":"host-a","os":"windows"}"#;
        let mut expected = Vec::new();
        expected.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        expected.extend_from_slice(payload);

        assert_eq!(frame, expected);
    }
}
