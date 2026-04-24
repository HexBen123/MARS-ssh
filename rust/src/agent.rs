use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_yamux::StreamHandle;

use crate::config::AgentConfig;
use crate::protocol::{read_json_frame, write_json_frame, Hello, Response};
use crate::state::current_utc_rfc3339;
use crate::tlsutil::{connect_pinned_tls, relay_host};

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
    let response: Response = read_json_frame(&mut ctrl)
        .await
        .context("read hello response")?;
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
    let banner = format!("ssh -p {} user@{}", response.assigned_port, public_host);
    eprintln!("=====================================================");
    eprintln!(" 已注册到中转 {}", cfg.relay);
    eprintln!(" AI 或用户现在可以这样连到本机：");
    eprintln!("     {banner}");
    eprintln!(" 进来的流量会桥接到 {}", cfg.local_addr);
    eprintln!("=====================================================");
    let _ = write_info_file(cfg, &response, &banner);

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

fn write_info_file(cfg: &AgentConfig, response: &Response, banner: &str) -> Result<()> {
    let content = format!(
        "最后成功注册时间：{}\n中转地址：   {}\n公开域名：   {}\n分配端口：   {}\nSSH 命令：   {}\n",
        current_local_time_string(),
        cfg.relay,
        response.public_host,
        response.assigned_port,
        banner
    );
    std::fs::write("agent-info.txt", content).context("write agent-info.txt")
}

fn current_local_time_string() -> String {
    current_utc_rfc3339()
}

async fn handle_stream(mut local_addr: String, mut stream: StreamHandle) -> Result<()> {
    if local_addr.is_empty() {
        local_addr = "127.0.0.1:22".to_string();
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
