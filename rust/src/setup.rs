pub fn sanitize_hostname(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            'a'..='z' | '0'..='9' | '-' => out.push(ch),
            'A'..='Z' => out.push(ch.to_ascii_lowercase()),
            _ => out.push('-'),
        }
    }
    if out.is_empty() {
        "agent".to_string()
    } else {
        out
    }
}

pub fn agent_id_from_hostname(hostname: &str, suffix: &[u8]) -> String {
    format!("{}-{}", sanitize_hostname(hostname), hex::encode(suffix))
}

pub fn generate_agent_id() -> anyhow::Result<String> {
    let hostname = hostname::get()
        .ok()
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "agent".to_string());
    let mut suffix = [0u8; 3];
    getrandom::getrandom(&mut suffix).map_err(|err| anyhow::anyhow!("generate agent id: {err}"))?;
    Ok(agent_id_from_hostname(&hostname, &suffix))
}

pub fn generate_token_hex() -> anyhow::Result<String> {
    let mut token = [0u8; 32];
    getrandom::getrandom(&mut token).map_err(|err| anyhow::anyhow!("generate token: {err}"))?;
    Ok(hex::encode(token))
}

pub fn generate_self_signed_cert(
    cert_path: &str,
    key_path: &str,
    hosts: &[String],
) -> anyhow::Result<()> {
    let certified = rcgen::generate_simple_self_signed(hosts.to_vec())?;
    std::fs::write(cert_path, certified.cert.pem())
        .map_err(|err| anyhow::anyhow!("write cert {cert_path}: {err}"))?;
    std::fs::write(key_path, certified.key_pair.serialize_pem())
        .map_err(|err| anyhow::anyhow!("write key {key_path}: {err}"))?;
    Ok(())
}
