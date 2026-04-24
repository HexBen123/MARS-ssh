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
