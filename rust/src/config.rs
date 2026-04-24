use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    pub relay: String,
    pub server_name: Option<String>,
    pub fingerprint: String,
    pub token: String,
    pub agent_id: String,
    #[serde(default = "default_local_addr")]
    pub local_addr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayConfig {
    pub listen: String,
    pub public_host: String,
    pub token: String,
    pub tls: TlsFiles,
    pub port_range: PortRange,
    #[serde(default = "default_state_file")]
    pub state_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsFiles {
    pub cert: String,
    pub key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRange {
    pub min: u16,
    pub max: u16,
}

fn default_local_addr() -> String {
    "127.0.0.1:22".to_string()
}

fn default_state_file() -> String {
    "state.json".to_string()
}

pub fn parse_agent_config(yaml: &str) -> Result<AgentConfig> {
    let yaml = yaml.trim_start_matches('\u{feff}');
    let mut cfg: AgentConfig = serde_yaml::from_str(yaml).context("parse agent yaml")?;
    if cfg.local_addr.is_empty() {
        cfg.local_addr = default_local_addr();
    }
    validate_agent(&cfg, true)?;
    Ok(cfg)
}

pub fn parse_agent_config_for_bootstrap(yaml: &str) -> Result<AgentConfig> {
    let yaml = yaml.trim_start_matches('\u{feff}');
    let mut cfg: AgentConfig = serde_yaml::from_str(yaml).context("parse agent yaml")?;
    if cfg.local_addr.is_empty() {
        cfg.local_addr = default_local_addr();
    }
    validate_agent(&cfg, false)?;
    Ok(cfg)
}

pub fn parse_relay_config(yaml: &str) -> Result<RelayConfig> {
    let yaml = yaml.trim_start_matches('\u{feff}');
    let cfg: RelayConfig = serde_yaml::from_str(yaml).context("parse relay yaml")?;
    validate_relay(&cfg)?;
    Ok(cfg)
}

pub fn load_agent_config(path: &str) -> Result<AgentConfig> {
    let content = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    parse_agent_config(&content)
}

pub fn load_agent_config_for_bootstrap(path: &str) -> Result<AgentConfig> {
    let content = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    parse_agent_config_for_bootstrap(&content)
}

pub fn load_relay_config(path: &str) -> Result<RelayConfig> {
    let content = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    parse_relay_config(&content)
}

pub fn save_agent_config(path: &str, cfg: &AgentConfig) -> Result<()> {
    let content = serde_yaml::to_string(cfg).context("marshal agent yaml")?;
    std::fs::write(path, content).with_context(|| format!("write {path}"))
}

pub fn save_relay_config(path: &str, cfg: &RelayConfig) -> Result<()> {
    let content = serde_yaml::to_string(cfg).context("marshal relay yaml")?;
    std::fs::write(path, content).with_context(|| format!("write {path}"))
}

pub fn validate_agent(cfg: &AgentConfig, require_fingerprint: bool) -> Result<()> {
    if cfg.relay.is_empty() {
        anyhow::bail!("relay is required (host:port)");
    }
    if cfg.token.is_empty() {
        anyhow::bail!("token is required");
    }
    if cfg.agent_id.is_empty() {
        anyhow::bail!("agent_id is required");
    }
    if require_fingerprint {
        if cfg.fingerprint.is_empty() {
            anyhow::bail!("fingerprint is empty");
        }
        if !cfg.fingerprint.starts_with("sha256:") {
            anyhow::bail!("fingerprint must start with sha256:");
        }
        crate::tlsutil::parse_fingerprint(&cfg.fingerprint)?;
    }
    Ok(())
}

pub fn validate_relay(cfg: &RelayConfig) -> Result<()> {
    if cfg.listen.is_empty() {
        anyhow::bail!("listen is required");
    }
    if cfg.token.is_empty() {
        anyhow::bail!("token is required");
    }
    if cfg.tls.cert.is_empty() || cfg.tls.key.is_empty() {
        anyhow::bail!("tls.cert and tls.key are required");
    }
    if cfg.port_range.min < 1024 || cfg.port_range.min > cfg.port_range.max {
        anyhow::bail!("port_range.min/max invalid");
    }
    Ok(())
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
}
