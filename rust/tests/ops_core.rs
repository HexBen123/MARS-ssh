use std::fs;
use std::path::PathBuf;

use mars_agent_rs::{logsink, menu, pubip, service};

fn target_path(parts: &[&str]) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    for part in parts {
        path.push(part);
    }
    path
}

#[test]
fn redacts_long_token_when_viewing_config_text() {
    let redacted = menu::redact_config_text("token: 1234567890abcdef\nlisten: :7000\n");
    assert!(redacted.contains("token: 12345678..."));
    assert!(redacted.contains("listen: :7000"));
}

#[test]
fn extracts_ipv4_from_plain_or_wrapped_public_ip_response() {
    assert_eq!(pubip::extract_ipv4("8.8.8.8\n").as_deref(), Some("8.8.8.8"));
    assert_eq!(
        pubip::extract_ipv4("当前 IP： 203.0.113.9 来自测试").as_deref(),
        Some("203.0.113.9")
    );
    assert_eq!(pubip::extract_ipv4("no ip here"), None);
}

#[test]
fn renders_linux_systemd_unit_with_quoted_exec_start() {
    let spec = service::Spec {
        name: "mars-agent".to_string(),
        display_name: "MARS Agent".to_string(),
        description: "MARS 目标代理".to_string(),
        bin_path: "/usr/local/bin/agent".to_string(),
        config_path: "/etc/mars/agent.yaml".to_string(),
        args: vec![
            "run".to_string(),
            "-config".to_string(),
            "/etc/mars/agent.yaml".to_string(),
        ],
        user: "mars".to_string(),
        group: "mars".to_string(),
    };

    let unit = service::render_systemd_unit(&spec);
    assert!(unit.contains("Description=MARS 目标代理"));
    assert!(unit.contains(
        "ExecStart=\"/usr/local/bin/agent\" \"run\" \"-config\" \"/etc/mars/agent.yaml\""
    ));
    assert!(unit.contains("Restart=always"));
    assert!(unit.contains("User=mars"));
    assert!(unit.contains("Group=mars"));
}

#[test]
fn extracts_windows_exe_path_from_service_command_line() {
    assert_eq!(
        service::extract_exe_path("\"C:\\Program Files\\MARS\\agent.exe\" run -config agent.yaml"),
        Some("C:\\Program Files\\MARS\\agent.exe".to_string())
    );
    assert_eq!(
        service::extract_exe_path("C:\\MARS\\agent.exe run"),
        Some("C:\\MARS\\agent.exe".to_string())
    );
}

#[test]
fn logsink_rotates_when_existing_file_exceeds_cap() {
    let dir = target_path(&["test-logs", &std::process::id().to_string()]);
    fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("agent.yaml");
    fs::write(&cfg, "").unwrap();
    let log = dir.join("mars-agent.log");
    fs::write(&log, "0123456789abcdef").unwrap();

    logsink::setup_with_size_cap(cfg.to_str().unwrap(), "mars-agent.log", 8).unwrap();
    mars_agent_rs::mars_log!("hello {}", "log");

    assert_eq!(
        fs::read_to_string(log.with_extension("log.1")).unwrap(),
        "0123456789abcdef"
    );
    assert!(fs::read_to_string(&log).unwrap().contains("hello log"));
}
