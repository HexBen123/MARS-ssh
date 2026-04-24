use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_FRAME_LEN: usize = 64 * 1024;

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

#[cfg(test)]
mod tests {
    use super::*;

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
