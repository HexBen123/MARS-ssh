use std::path::PathBuf;

use mars_agent_rs::{config::parse_agent_config_for_bootstrap, setup, tlsutil};

fn fixture_path(name: &str) -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push(name);
    path.to_string_lossy().to_string()
}

#[test]
fn sanitizes_hostname_like_go_agent_wizard() {
    assert_eq!(setup::sanitize_hostname("My PC_01"), "my-pc-01");
    assert_eq!(setup::sanitize_hostname(""), "agent");
    assert_eq!(setup::sanitize_hostname("中文主机"), "----");
}

#[test]
fn builds_agent_id_from_sanitized_hostname_and_hex_suffix() {
    assert_eq!(
        setup::agent_id_from_hostname("My PC", &[0xab, 0xcd, 0xef]),
        "my-pc-abcdef"
    );
}

#[test]
fn bootstrap_agent_config_allows_missing_fingerprint() {
    let cfg = parse_agent_config_for_bootstrap(
        r#"
relay: relay.example.com:7000
server_name: relay.example.com
fingerprint: ""
token: secret
agent_id: node-a
local_addr: 127.0.0.1:22
"#,
    )
    .unwrap();

    assert_eq!(cfg.relay, "relay.example.com:7000");
    assert_eq!(cfg.fingerprint, "");
}

#[test]
fn computes_sha256_fingerprint_from_certificate_file() {
    let fingerprint = tlsutil::fingerprint_from_file(&fixture_path("relay_cert.pem")).unwrap();
    assert_eq!(
        fingerprint,
        "sha256:74b49e8e666e83cacb4c8e19cba2d12045ef49e25e6ab6e324d628e57ccf81df"
    );
}

#[test]
fn generates_random_relay_token_as_32_bytes_hex() {
    let token = setup::generate_token_hex().unwrap();
    assert_eq!(token.len(), 64);
    assert!(token.chars().all(|ch| ch.is_ascii_hexdigit()));
}

#[test]
fn generates_self_signed_certificate_loadable_by_rust_relay() {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.push("target");
    dir.push("test-certs");
    std::fs::create_dir_all(&dir).unwrap();
    let cert = dir.join(format!("relay-{}.pem", std::process::id()));
    let key = dir.join(format!("relay-{}.key", std::process::id()));

    setup::generate_self_signed_cert(
        cert.to_str().unwrap(),
        key.to_str().unwrap(),
        &["127.0.0.1".to_string()],
    )
    .unwrap();

    let fingerprint = tlsutil::fingerprint_from_file(cert.to_str().unwrap()).unwrap();
    assert!(fingerprint.starts_with("sha256:"));
    mars_agent_rs::tlsutil::load_server_tls_acceptor(cert.to_str().unwrap(), key.to_str().unwrap())
        .unwrap();
}
