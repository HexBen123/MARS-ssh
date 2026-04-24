use mars_agent_rs::{
    config::{parse_relay_config, PortRange, RelayConfig, TlsFiles},
    portpool::PortPool,
    state::{Entry, Store},
};

#[test]
fn parses_go_relay_yaml_and_defaults_state_file() {
    let cfg = parse_relay_config(
        r#"
listen: :7000
public_host: relay.example.com
token: secret
tls:
  cert: cert.pem
  key: key.pem
port_range:
  min: 20000
  max: 20002
"#,
    )
    .unwrap();

    assert_eq!(
        cfg,
        RelayConfig {
            listen: ":7000".to_string(),
            public_host: "relay.example.com".to_string(),
            token: "secret".to_string(),
            tls: TlsFiles {
                cert: "cert.pem".to_string(),
                key: "key.pem".to_string(),
            },
            port_range: PortRange {
                min: 20000,
                max: 20002,
            },
            state_file: "state.json".to_string(),
        }
    );
}

#[test]
fn port_pool_reserves_allocates_and_releases_like_go() {
    let mut pool = PortPool::new(20000, 20002);

    assert!(pool.reserve(20001));
    assert!(!pool.reserve(20001));
    assert!(!pool.reserve(19999));
    assert_eq!(pool.allocate().unwrap(), 20000);
    assert_eq!(pool.allocate().unwrap(), 20002);
    assert!(pool.allocate().is_err());

    pool.release(20001);
    assert_eq!(pool.allocate().unwrap(), 20001);
}

#[test]
fn state_store_loads_and_saves_go_json_schema() {
    let json = r#"
{
  "agents": {
    "node-a": {
      "port": 20000,
      "hostname": "host-a",
      "last_seen": "2026-04-24T03:00:00Z"
    }
  }
}
"#;

    let mut store = Store::from_json(json).unwrap();
    let existing = store.get("node-a").unwrap();
    assert_eq!(existing.port, 20000);
    assert_eq!(existing.hostname.as_deref(), Some("host-a"));

    store.put(
        "node-b",
        Entry {
            port: 20001,
            hostname: Some("host-b".to_string()),
            last_seen: None,
        },
    );

    let saved = store.to_json_pretty().unwrap();
    assert!(saved.contains("\"node-a\""));
    assert!(saved.contains("\"node-b\""));
    assert!(saved.contains("\"port\": 20001"));
    assert!(saved.contains("\"hostname\": \"host-b\""));
    assert!(saved.contains("\"last_seen\""));
}
