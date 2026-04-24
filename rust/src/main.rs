use anyhow::{anyhow, Result};

const USAGE: &str = r#"MARS agent-rs —— Rust 兼容性验证客户端

用法：
  agent-rs [-config <路径>]              启动（需要已有 agent.yaml）
  agent-rs run [-config <路径>]          同上
  agent-rs help                          显示帮助

说明：
  当前 Rust 版本用于验证体积和协议兼容性；ms/install/uninstall 还未迁移。
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
