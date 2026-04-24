use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, Context, Result};

const USAGE: &str = r#"MARS agent-rs —— Rust 反向 SSH 隧道目标端

用法：
  agent-rs [-config <路径>]              启动（首次运行进入交互向导）
  agent-rs run [-config <路径>]          同上
  agent-rs help                          显示帮助

说明：
  当前 Rust agent 已迁移运行主链路和首次运行向导；ms/install/uninstall 将在后续阶段迁移。
"#;

#[tokio::main]
async fn main() {
    if let Err(err) = real_main().await {
        eprintln!("agent-rs 失败：{err:#}");
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
                run_agent_wizard(&cfg_path, None).await?;
            }
            let cfg = mars_agent_rs::load_agent_config(&cfg_path)?;
            mars_agent_rs::run_forever(cfg).await
        }
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(())
        }
        "ms" | "menu" | "install" | "uninstall" => {
            Err(anyhow!("Rust 验证版尚未迁移 `{cmd}`；请继续使用 Go agent"))
        }
        other => Err(anyhow!("未知命令 {other:?}\n\n{USAGE}")),
    }
}

fn parse_config_path(args: &[String]) -> Result<String> {
    let mut cfg = "agent.yaml".to_string();
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

async fn run_agent_wizard(
    cfg_path: &str,
    existing: Option<&mars_agent_rs::config::AgentConfig>,
) -> Result<()> {
    println!("=====================================================");
    if existing.is_some() {
        println!(" MARS 目标代理 —— 修改配置");
    } else {
        println!(" MARS 目标代理 —— 首次启动向导");
    }
    println!(" （直接回车使用方括号里的默认值）");
    println!("=====================================================");

    let default_relay = existing.map(|cfg| cfg.relay.as_str()).unwrap_or("");
    let default_token = existing.map(|cfg| cfg.token.as_str()).unwrap_or("");
    let default_local = existing
        .map(|cfg| cfg.local_addr.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("127.0.0.1:22");

    let relay_addr = prompt_string("中转地址（host:port）", default_relay)?;
    if relay_addr.is_empty() {
        anyhow::bail!("必须填写中转地址");
    }
    let server_name = mars_agent_rs::tlsutil::relay_host(&relay_addr)
        .ok_or_else(|| anyhow!("中转地址必须是 host:port 格式"))?;

    let token = prompt_string("令牌（从中转方复制过来）", default_token)?;
    if token.is_empty() {
        anyhow::bail!("必须填写令牌");
    }

    let local_addr = prompt_string("要暴露的本地服务地址", default_local)?;
    let agent_id = existing
        .map(|cfg| cfg.agent_id.clone())
        .filter(|value| !value.is_empty())
        .map(Ok)
        .unwrap_or_else(mars_agent_rs::setup::generate_agent_id)?;

    let dir = std::path::Path::new(cfg_path)
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("创建目录 {} 失败", dir.display()))?;

    let mut fingerprint = existing
        .map(|cfg| cfg.fingerprint.clone())
        .unwrap_or_default();
    if fingerprint.is_empty() || existing.map(|cfg| cfg.relay.as_str()) != Some(relay_addr.as_str())
    {
        print!("正在从 {relay_addr} 获取 TLS 指纹 ... ");
        let _ = std::io::stdout().flush();
        match mars_agent_rs::tlsutil::fetch_fingerprint(&relay_addr, &server_name).await {
            Ok(value) => {
                println!("完成");
                println!("  已钉扎：{value}");
                fingerprint = value;
            }
            Err(err) => {
                println!("失败");
                return Err(err).context("获取指纹失败");
            }
        }
    }

    let cfg = mars_agent_rs::config::AgentConfig {
        relay: relay_addr,
        server_name: Some(server_name),
        fingerprint,
        token,
        agent_id: agent_id.clone(),
        local_addr,
    };
    mars_agent_rs::config::save_agent_config(cfg_path, &cfg)
        .with_context(|| format!("保存配置 {cfg_path} 失败"))?;

    println!();
    println!("=====================================================");
    println!(" 配置已保存到 {cfg_path} （agent_id={agent_id}）");
    if existing.is_none() {
        println!(" 小贴士：以后想改配置 / 启停服务，跑 `<本程序路径> ms`");
        println!("        `sudo <本程序> install` 之后，`ms` 会成为全局命令。");
        println!(" 正在连接中转 ...");
    }
    println!("=====================================================");
    println!();
    Ok(())
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
