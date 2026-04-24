use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use futures_util::StreamExt;
use mars_agent_rs::{
    config::{AgentConfig, PortRange, RelayConfig, TlsFiles},
    protocol::{read_json_frame, write_json_frame, Hello, Response},
    relay::{normalize_listen_addr, Server},
    state::Store,
    tlsutil::connect_pinned_tls,
};
use tokio::io::AsyncWriteExt;
use tokio_yamux::{Config as YamuxConfig, Session};

fn test_state_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push("test-state");
    fs::create_dir_all(&path).unwrap();
    path.push(format!("{name}-{}.json", std::process::id()));
    path
}

fn relay_config(state_file: String) -> RelayConfig {
    RelayConfig {
        listen: ":7000".to_string(),
        public_host: "relay.example.com".to_string(),
        token: "secret".to_string(),
        tls: TlsFiles {
            cert: "cert.pem".to_string(),
            key: "key.pem".to_string(),
        },
        port_range: PortRange {
            min: 23000,
            max: 23001,
        },
        state_file,
    }
}

fn fixture_path(name: &str) -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push(name);
    path.to_string_lossy().to_string()
}

fn hello(token: &str, agent_id: &str) -> Hello {
    Hello {
        kind: "hello".to_string(),
        token: token.to_string(),
        agent_id: agent_id.to_string(),
        hostname: "host-a".to_string(),
        os: "windows".to_string(),
    }
}

#[test]
fn normalizes_go_colon_listen_addr_for_rust_tcp_listener() {
    assert_eq!(normalize_listen_addr(":7000"), "0.0.0.0:7000");
    assert_eq!(normalize_listen_addr("127.0.0.1:7000"), "127.0.0.1:7000");
}

#[test]
fn relay_registration_validates_token_and_persists_sticky_port() {
    let state_path = test_state_path("sticky");
    let state_file = state_path.to_string_lossy().to_string();
    let cfg = relay_config(state_file.clone());
    let server = Server::new(cfg.clone(), Store::empty());

    let rejected = server
        .register_hello(&hello("bad-token", "node-a"))
        .unwrap();
    assert_eq!(rejected.kind, "err");
    assert_eq!(rejected.reason, "unauthorized");

    let accepted = server.register_hello(&hello("secret", "node-a")).unwrap();
    assert_eq!(accepted.kind, "ok");
    assert_eq!(accepted.assigned_port, 23000);
    assert_eq!(accepted.public_host, "relay.example.com");

    let saved = fs::read_to_string(&state_path).unwrap();
    assert!(saved.contains("\"node-a\""));
    assert!(saved.contains("\"port\": 23000"));

    let restarted = Server::new(cfg, Store::load(&state_file).unwrap());
    let sticky = restarted
        .register_hello(&hello("secret", "node-a"))
        .unwrap();
    assert_eq!(sticky.kind, "ok");
    assert_eq!(sticky.assigned_port, 23000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rust_relay_accepts_tls_yamux_hello_and_returns_assigned_port() -> anyhow::Result<()> {
    let state_path = test_state_path("handshake");
    let public_probe = std::net::TcpListener::bind("127.0.0.1:0")?;
    let public_port = public_probe.local_addr()?.port();
    drop(public_probe);

    let cfg = RelayConfig {
        listen: "127.0.0.1:0".to_string(),
        public_host: "127.0.0.1".to_string(),
        token: "secret".to_string(),
        tls: TlsFiles {
            cert: fixture_path("relay_cert.pem"),
            key: fixture_path("relay_key.pem"),
        },
        port_range: PortRange {
            min: public_port,
            max: public_port,
        },
        state_file: state_path.to_string_lossy().to_string(),
    };
    let bound = Server::new(cfg, Store::empty()).bind().await?;
    let relay_addr = bound.local_addr();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(bound.run_until_shutdown(async {
        let _ = shutdown_rx.await;
    }));

    let agent_cfg = AgentConfig {
        relay: relay_addr.to_string(),
        server_name: Some("127.0.0.1".to_string()),
        fingerprint: "sha256:74b49e8e666e83cacb4c8e19cba2d12045ef49e25e6ab6e324d628e57ccf81df"
            .to_string(),
        token: "secret".to_string(),
        agent_id: "node-integration".to_string(),
        local_addr: "127.0.0.1:22".to_string(),
    };
    let tls = connect_pinned_tls(&agent_cfg).await?;
    let mut session = Session::new_client(tls, YamuxConfig::default());
    let mut control = session.control();
    let session_task = tokio::spawn(async move {
        while let Some(next) = session.next().await {
            next?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let mut ctrl = control.open_stream().await?;
    write_json_frame(&mut ctrl, &hello("secret", "node-integration")).await?;
    let response: Response = read_json_frame(&mut ctrl).await?;
    let _ = ctrl.shutdown().await;
    control.close().await;
    let _ = session_task.await?;
    let _ = shutdown_tx.send(());
    tokio::time::timeout(Duration::from_secs(2), server_task).await???;

    assert_eq!(response.kind, "ok");
    assert_eq!(response.assigned_port, public_port);
    assert_eq!(response.public_host, "127.0.0.1");
    Ok(())
}
