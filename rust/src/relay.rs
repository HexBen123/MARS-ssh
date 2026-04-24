use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_yamux::{Config as YamuxConfig, Control, Session, StreamHandle};

use crate::config::RelayConfig;
use crate::portpool::PortPool;
use crate::protocol::{read_json_frame, write_json_frame, Hello, Response};
use crate::state::{Entry, Store};
use crate::tlsutil::load_server_tls_acceptor;

#[derive(Clone)]
pub struct Server {
    cfg: Arc<RelayConfig>,
    inner: Arc<Mutex<RelayState>>,
}

struct RelayState {
    pool: PortPool,
    store: Store,
    sessions: HashMap<String, AgentSession>,
}

struct AgentSession {
    port: u16,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

pub struct BoundServer {
    server: Server,
    listener: TcpListener,
    local_addr: SocketAddr,
    acceptor: tokio_rustls::TlsAcceptor,
}

impl Server {
    pub fn new(cfg: RelayConfig, store: Store) -> Self {
        let mut pool = PortPool::new(cfg.port_range.min, cfg.port_range.max);
        for entry in store.agents.values() {
            pool.reserve(entry.port);
        }
        Self {
            cfg: Arc::new(cfg),
            inner: Arc::new(Mutex::new(RelayState {
                pool,
                store,
                sessions: HashMap::new(),
            })),
        }
    }

    pub fn register_hello(&self, hello: &Hello) -> Result<Response> {
        if hello.kind != "hello" || hello.token != self.cfg.token || hello.agent_id.is_empty() {
            return Ok(Response {
                kind: "err".to_string(),
                reason: "unauthorized".to_string(),
                assigned_port: 0,
                public_host: String::new(),
            });
        }

        let mut inner = self.lock_inner()?;
        let previous_port = inner.store.get(&hello.agent_id).map(|entry| entry.port);
        let port = if let Some(previous_port) = previous_port {
            inner.pool.reserve(previous_port);
            previous_port
        } else {
            inner.pool.allocate()?
        };

        inner.store.put(
            hello.agent_id.clone(),
            Entry {
                port,
                hostname: if hello.hostname.is_empty() {
                    None
                } else {
                    Some(hello.hostname.clone())
                },
                last_seen: None,
            },
        );
        let store = inner.store.clone();
        drop(inner);

        if let Err(err) = store.save(&self.cfg.state_file) {
            eprintln!("persist relay state failed: {err:#}");
        }

        Ok(Response {
            kind: "ok".to_string(),
            reason: String::new(),
            assigned_port: port,
            public_host: self.cfg.public_host.clone(),
        })
    }

    pub async fn bind(self) -> Result<BoundServer> {
        let acceptor = load_server_tls_acceptor(&self.cfg.tls.cert, &self.cfg.tls.key)?;
        let listen = normalize_listen_addr(&self.cfg.listen);
        let listener = TcpListener::bind(&listen)
            .await
            .with_context(|| format!("listen {listen}"))?;
        let local_addr = listener.local_addr().context("read relay local addr")?;
        Ok(BoundServer {
            server: self,
            listener,
            local_addr,
            acceptor,
        })
    }

    pub async fn run_until_shutdown<F>(self, shutdown: F) -> Result<()>
    where
        F: Future<Output = ()>,
    {
        self.bind().await?.run_until_shutdown(shutdown).await
    }

    fn lock_inner(&self) -> Result<std::sync::MutexGuard<'_, RelayState>> {
        self.inner
            .lock()
            .map_err(|_| anyhow::anyhow!("relay state mutex poisoned"))
    }

    fn replace_session(
        &self,
        agent_id: &str,
        port: u16,
    ) -> Result<tokio::sync::oneshot::Receiver<()>> {
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        let mut inner = self.lock_inner()?;
        if let Some(prev) = inner.sessions.remove(agent_id) {
            let _ = prev.shutdown.send(());
        }
        inner
            .sessions
            .insert(agent_id.to_string(), AgentSession { port, shutdown });
        Ok(shutdown_rx)
    }

    fn remove_session(&self, agent_id: &str, port: u16) {
        if let Ok(mut inner) = self.lock_inner() {
            if inner
                .sessions
                .get(agent_id)
                .map(|session| session.port == port)
                .unwrap_or(false)
            {
                inner.sessions.remove(agent_id);
            }
        }
    }

    fn release_port(&self, port: u16) {
        if let Ok(mut inner) = self.lock_inner() {
            inner.pool.release(port);
        }
    }
}

impl BoundServer {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn run_until_shutdown<F>(self, shutdown: F) -> Result<()>
    where
        F: Future<Output = ()>,
    {
        eprintln!(
            "中转已监听 {}（TLS）；公开地址 {:?}；端口池 {}-{}",
            self.local_addr,
            self.server.cfg.public_host,
            self.server.cfg.port_range.min,
            self.server.cfg.port_range.max
        );
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                _ = &mut shutdown => return Ok(()),
                accepted = self.listener.accept() => {
                    let (tcp, remote) = accepted.context("accept relay connection")?;
                    let server = self.server.clone();
                    let acceptor = self.acceptor.clone();
                    tokio::spawn(async move {
                        if let Err(err) = server.handle_conn(tcp, remote, acceptor).await {
                            eprintln!("[{remote}] relay connection ended: {err:#}");
                        }
                    });
                }
            }
        }
    }
}

impl Server {
    async fn handle_conn(
        &self,
        tcp: TcpStream,
        remote: SocketAddr,
        acceptor: tokio_rustls::TlsAcceptor,
    ) -> Result<()> {
        let tls = timeout(Duration::from_secs(10), acceptor.accept(tcp))
            .await
            .context("tls handshake timeout")?
            .context("tls handshake")?;
        let mut session = Session::new_server(tls, YamuxConfig::default());
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

        let next_stream = timeout(Duration::from_secs(10), stream_rx.recv())
            .await
            .context("accept control stream timeout")?;
        let mut ctrl = match next_stream {
            Some(stream) => stream,
            None => anyhow::bail!("yamux closed before control stream"),
        };

        let handshake = async {
            let hello: Hello = timeout(Duration::from_secs(10), read_json_frame(&mut ctrl))
                .await
                .context("read hello timeout")?
                .context("read hello")?;
            let response = self.register_hello(&hello)?;
            write_json_frame(&mut ctrl, &response)
                .await
                .context("write hello response")?;
            let _ = ctrl.shutdown().await;
            Ok::<_, anyhow::Error>((hello, response))
        };
        let (hello, response) = match handshake.await {
            Ok(result) => result,
            Err(err) => {
                control.close().await;
                let _ = session_task.await;
                return Err(err);
            }
        };

        if response.kind != "ok" {
            control.close().await;
            let _ = session_task.await;
            return Ok(());
        }

        self.serve_agent(
            hello.agent_id,
            response.assigned_port,
            control,
            stream_rx,
            session_task,
            remote.to_string(),
        )
        .await
    }

    async fn serve_agent(
        &self,
        agent_id: String,
        port: u16,
        mut control: Control,
        mut stream_rx: tokio::sync::mpsc::Receiver<StreamHandle>,
        session_task: JoinHandle<Result<()>>,
        remote: String,
    ) -> Result<()> {
        let mut shutdown = self.replace_session(&agent_id, port)?;
        let bind_addr = format!("0.0.0.0:{port}");
        let listener = match bind_public_listener(&bind_addr).await {
            Ok(listener) => listener,
            Err(err) => {
                self.release_port(port);
                control.close().await;
                let _ = session_task.await;
                return Err(err);
            }
        };
        eprintln!("目标 {agent_id:?} 从 {remote} 接入，开放 {bind_addr}");

        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                incoming = stream_rx.recv() => {
                    match incoming {
                        Some(mut stream) => {
                            let _ = stream.shutdown().await;
                        }
                        None => break,
                    }
                }
                accepted = listener.accept() => {
                    let (client, _) = accepted.with_context(|| format!("accept public listener {bind_addr}"))?;
                    tokio::spawn(bridge(control.clone(), client, agent_id.clone()));
                }
            }
        }

        control.close().await;
        let _ = session_task.await;
        self.remove_session(&agent_id, port);
        eprintln!("目标 {agent_id:?} 已断开，关闭 {bind_addr}（端口保留以便重连）");
        Ok(())
    }
}

pub fn normalize_listen_addr(listen: &str) -> String {
    if let Some(port) = listen.strip_prefix(':') {
        format!("0.0.0.0:{port}")
    } else {
        listen.to_string()
    }
}

async fn bind_public_listener(addr: &str) -> Result<TcpListener> {
    let mut last_err = None;
    for _ in 0..20 {
        match TcpListener::bind(addr).await {
            Ok(listener) => return Ok(listener),
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(err) => return Err(err).with_context(|| format!("listen {addr}")),
        }
    }
    Err(last_err
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("listen {addr} failed")))
    .with_context(|| format!("listen {addr}"))
}

async fn bridge(mut control: Control, mut client: TcpStream, agent_id: String) {
    match control.open_stream().await {
        Ok(mut stream) => {
            if let Err(err) = tokio::io::copy_bidirectional(&mut client, &mut stream).await {
                eprintln!("[{agent_id}] stream bridge failed: {err:#}");
            }
            let _ = client.shutdown().await;
            let _ = stream.shutdown().await;
        }
        Err(err) => {
            eprintln!("[{agent_id}] open yamux stream failed: {err:#}");
            let _ = client.shutdown().await;
        }
    }
}
