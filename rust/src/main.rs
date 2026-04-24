use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, Context, Result};

const LOG_FILE_NAME: &str = "mars-agent.log";
const SERVICE_NAME: &str = "mars-agent";
const SERVICE_DISPLAY_NAME: &str = "MARS Reverse SSH Tunnel Agent";
const SERVICE_DESCRIPTION: &str = "MARS (Minimal AI Reverse Ssh) 目标代理";

const USAGE: &str = r#"MARS agent —— 反向 SSH 隧道目标端

用法：
  agent [-config <路径>]              启动（首次运行进入交互向导）
  agent run [-config <路径>]          同上
  agent ms [-config <路径>]           打开服务管理菜单
  agent install [-config <路径>]      注册为系统服务并启动
  agent uninstall                     停止服务并移除注册
"#;

#[cfg(windows)]
windows_service::define_windows_service!(ffi_service_main, agent_service_main);

#[tokio::main]
async fn main() {
    if let Err(err) = real_main().await {
        mars_agent_rs::mars_log!("agent 失败：{err:#}");
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
            #[cfg(windows)]
            {
                if mars_agent_rs::service::try_start_dispatcher(SERVICE_NAME, ffi_service_main)? {
                    return Ok(());
                }
            }
            ensure_agent_config(&cfg_path, false).await?;
            run_agent_foreground(cfg_path).await
        }
        "ms" | "menu" => {
            let cfg_path = absolute_config_path(&parse_config_path(&args)?)?;
            ensure_agent_config(&cfg_path, false).await?;
            run_agent_menu(cfg_path).await
        }
        "install" => {
            let cfg_path = absolute_config_path(&parse_config_path(&args)?)?;
            ensure_agent_config(&cfg_path, false).await?;
            install_agent_service(cfg_path)
        }
        "uninstall" => {
            mars_agent_rs::service::uninstall(SERVICE_NAME)?;
            println!("服务 {SERVICE_NAME:?} 已移除");
            Ok(())
        }
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(anyhow!("未知命令 {other:?}\n\n{USAGE}")),
    }
}

#[cfg(windows)]
fn agent_service_main(_arguments: Vec<std::ffi::OsString>) {
    if let Err(err) = agent_service_entry() {
        mars_agent_rs::mars_log!("agent service failed: {err:#}");
    }
}

#[cfg(windows)]
fn agent_service_entry() -> Result<()> {
    let args = std::env::args().skip(2).collect::<Vec<_>>();
    let cfg_path = parse_config_path(&args)?;
    mars_agent_rs::service::run_windows_service(SERVICE_NAME, |shutdown| async move {
        run_agent_until(cfg_path, async {
            let _ = shutdown.await;
        })
        .await
    })
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

async fn ensure_agent_config(cfg_path: &str, running_as_service: bool) -> Result<()> {
    if mars_agent_rs::setup::should_run_interactive_wizard(
        Path::new(cfg_path).exists(),
        running_as_service,
    ) {
        run_agent_wizard(cfg_path, None).await?;
    }
    Ok(())
}

async fn run_agent_foreground(cfg_path: String) -> Result<()> {
    run_agent_until(cfg_path, wait_for_shutdown()).await
}

async fn run_agent_until<S>(cfg_path: String, shutdown: S) -> Result<()>
where
    S: std::future::Future<Output = ()>,
{
    if let Err(err) = mars_agent_rs::logsink::setup(&cfg_path, LOG_FILE_NAME) {
        mars_agent_rs::mars_log!("提示：无法打开日志文件（{err:#}），仅输出到 stderr");
    }
    let cfg = mars_agent_rs::load_agent_config(&cfg_path)?;
    tokio::pin!(shutdown);
    tokio::select! {
        result = mars_agent_rs::run_forever(cfg) => result,
        _ = &mut shutdown => Ok(()),
    }
}

async fn wait_for_shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn run_agent_menu(cfg_path: String) -> Result<()> {
    let bin = std::env::current_exe()
        .context("定位自身可执行文件失败")?
        .to_string_lossy()
        .to_string();
    let install_spec = agent_service_spec(bin, cfg_path.clone());
    let edit_path = cfg_path.clone();
    let run_path = cfg_path.clone();
    mars_agent_rs::menu::run(mars_agent_rs::menu::Spec {
        title: "MARS 目标代理（agent）".to_string(),
        service_name: SERVICE_NAME.to_string(),
        config_path: cfg_path,
        install_spec,
        edit_config: Box::new(move || {
            let path = edit_path.clone();
            Box::pin(async move {
                let cur = mars_agent_rs::config::load_agent_config_for_bootstrap(&path)?;
                run_agent_wizard(&path, Some(&cur)).await
            })
        }),
        run_foreground: Some(Box::new(move || {
            let path = run_path.clone();
            Box::pin(async move { run_agent_foreground(path).await })
        })),
    })
    .await
}

fn install_agent_service(cfg_path: String) -> Result<()> {
    let bin = std::env::current_exe()
        .context("定位自身可执行文件失败")?
        .to_string_lossy()
        .to_string();
    mars_agent_rs::load_agent_config(&cfg_path)?;
    mars_agent_rs::service::install(agent_service_spec(bin, cfg_path.clone()))?;
    println!("服务 {SERVICE_NAME:?} 已注册并启动（配置：{cfg_path}）");
    println!("现在可以在任何地方直接敲 `ms` 打开管理菜单。");
    Ok(())
}

fn agent_service_spec(bin: String, cfg_path: String) -> mars_agent_rs::service::Spec {
    mars_agent_rs::service::Spec {
        name: SERVICE_NAME.to_string(),
        display_name: SERVICE_DISPLAY_NAME.to_string(),
        description: SERVICE_DESCRIPTION.to_string(),
        bin_path: bin,
        config_path: cfg_path.clone(),
        args: vec!["run".to_string(), "-config".to_string(), cfg_path],
        user: String::new(),
        group: String::new(),
    }
}

fn absolute_config_path(path: &str) -> Result<String> {
    let path = Path::new(path);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(abs.to_string_lossy().to_string())
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
