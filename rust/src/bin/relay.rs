use anyhow::{anyhow, Result};

const USAGE: &str = r#"MARS relay-rs —— Rust 公网侧反向隧道中转

用法：
  relay [-config <路径>]              启动（需要已有 relay.yaml）
  relay run [-config <路径>]          同上
  relay help                          显示帮助

说明：
  当前 Rust relay 已迁移运行主链路；ms/install/uninstall 将在后续阶段迁移。
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
