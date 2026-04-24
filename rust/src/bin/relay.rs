use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, Context, Result};

const USAGE: &str = r#"MARS relay-rs —— Rust 公网侧反向隧道中转

用法：
  relay [-config <路径>]              启动（首次运行进入交互向导）
  relay run [-config <路径>]          同上
  relay help                          显示帮助

说明：
  当前 Rust relay 已迁移运行主链路和首次运行向导；ms/install/uninstall 将在后续阶段迁移。
"#;

#[tokio::main]
async fn main() {
    if let Err(err) = real_main().await {
        eprintln!("relay-rs 失败：{err:#}");
        std::process::exit(1);
    }
}

async fn real_main() -> Result<()> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut cmd = "run".to_string();
    if let Some(first) = args.first() {
        if !first.starts_with('-') {
            cmd = args.remove(0);
        }
    }

    match cmd.as_str() {
        "run" => {
            let cfg_path = parse_config_path(&args)?;
            if !Path::new(&cfg_path).exists() {
                run_relay_wizard(&cfg_path, None)?;
            }
            let cfg = mars_agent_rs::config::load_relay_config(&cfg_path)?;
            let store = mars_agent_rs::state::Store::load(&cfg.state_file)?;
            let server = mars_agent_rs::relay::Server::new(cfg, store);
            server.run_until_shutdown(wait_for_shutdown()).await
        }
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(())
        }
        "ms" | "menu" | "install" | "uninstall" => {
            Err(anyhow!("Rust relay 尚未迁移 `{cmd}`；请继续使用 Go relay"))
        }
        other => Err(anyhow!("未知命令 {other:?}\n\n{USAGE}")),
    }
}

async fn wait_for_shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

fn parse_config_path(args: &[String]) -> Result<String> {
    let mut cfg = "relay.yaml".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-config" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("-config requires a path"))?;
                cfg = value.clone();
                i += 2;
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(anyhow!("未知参数 {other:?}")),
        }
    }
    Ok(cfg)
}

fn run_relay_wizard(
    cfg_path: &str,
    existing: Option<&mars_agent_rs::config::RelayConfig>,
) -> Result<()> {
    println!("=====================================================");
    if existing.is_some() {
        println!(" MARS 中转 —— 修改配置");
    } else {
        println!(" MARS 中转 —— 首次启动向导");
    }
    println!(" （直接回车使用方括号里的默认值）");
    println!("=====================================================");

    let default_port = existing
        .and_then(|cfg| port_from_listen(&cfg.listen))
        .unwrap_or(7000);
    let default_host = existing.map(|cfg| cfg.public_host.as_str()).unwrap_or("");
    let default_min = existing.map(|cfg| cfg.port_range.min).unwrap_or(20000);
    let default_max = existing.map(|cfg| cfg.port_range.max).unwrap_or(21000);

    let port = prompt_int("控制端口（agent 用于拨入）", default_port, 1, 65535)?;
    let host = prompt_string("对外公开的域名或 IP", default_host)?;
    if host.is_empty() {
        anyhow::bail!("必须填写对外公开的域名或 IP");
    }
    let min_port = prompt_int("可分配端口范围 —— 起始", default_min, 1024, 65535)?;
    let max_port = prompt_int("可分配端口范围 —— 结束", default_max, min_port, 65535)?;

    let dir = Path::new(cfg_path)
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("创建目录 {} 失败", dir.display()))?;

    let cert_path = existing
        .and_then(|cfg| (!cfg.tls.cert.is_empty()).then(|| cfg.tls.cert.clone()))
        .unwrap_or_else(|| dir.join("cert.pem").to_string_lossy().to_string());
    let key_path = existing
        .and_then(|cfg| (!cfg.tls.key.is_empty()).then(|| cfg.tls.key.clone()))
        .unwrap_or_else(|| dir.join("key.pem").to_string_lossy().to_string());
    let state_path = existing
        .and_then(|cfg| (!cfg.state_file.is_empty()).then(|| cfg.state_file.clone()))
        .unwrap_or_else(|| dir.join("state.json").to_string_lossy().to_string());

    if existing.is_none() || !Path::new(&cert_path).exists() || !Path::new(&key_path).exists() {
        print!("正在生成自签证书 {cert_path} ... ");
        let _ = std::io::stdout().flush();
        mars_agent_rs::setup::generate_self_signed_cert(
            &cert_path,
            &key_path,
            std::slice::from_ref(&host),
        )?;
        println!("完成");
    }

    let token = existing
        .map(|cfg| cfg.token.clone())
        .filter(|value| !value.is_empty())
        .map(Ok)
        .unwrap_or_else(mars_agent_rs::setup::generate_token_hex)?;

    let cfg = mars_agent_rs::config::RelayConfig {
        listen: format!(":{port}"),
        public_host: host.clone(),
        token: token.clone(),
        tls: mars_agent_rs::config::TlsFiles {
            cert: cert_path,
            key: key_path,
        },
        port_range: mars_agent_rs::config::PortRange {
            min: min_port,
            max: max_port,
        },
        state_file: state_path,
    };
    mars_agent_rs::config::save_relay_config(cfg_path, &cfg)
        .with_context(|| format!("保存配置 {cfg_path} 失败"))?;

    println!();
    println!("=====================================================");
    if existing.is_some() {
        println!(" 配置已更新。把下面两行发给目标机操作者：");
    } else {
        println!(" 配置完成。把下面两行发给目标机操作者：");
    }
    println!("=====================================================");
    println!("   中转地址 ： {host}:{port}");
    println!("   令牌     ： {token}");
    println!("=====================================================");
    println!(" 配置已保存到 {cfg_path}");
    if existing.is_none() {
        println!(" 小贴士：以后想改配置 / 启停服务，跑 `<本程序路径> ms`");
        println!("        `sudo <本程序> install` 之后，`ms` 会成为全局命令。");
        println!(" 正在启动中转 ...");
    }
    println!();
    Ok(())
}

fn port_from_listen(listen: &str) -> Option<u16> {
    let (_, port) = listen.rsplit_once(':')?;
    port.parse().ok()
}

fn prompt_string(label: &str, default: &str) -> Result<String> {
    if default.is_empty() {
        print!("{label}: ");
    } else {
        print!("{label} [{default}]: ");
    }
    std::io::stdout().flush().context("flush stdout")?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("read stdin")?;
    let value = line.trim().to_string();
    if value.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(value)
    }
}

fn prompt_int(label: &str, default: u16, min: u16, max: u16) -> Result<u16> {
    loop {
        let value = prompt_string(label, &default.to_string())?;
        if let Ok(parsed) = value.parse::<u16>() {
            if parsed >= min && parsed <= max {
                return Ok(parsed);
            }
        }
        println!("  请输入 {min} 到 {max} 之间的整数");
    }
}
