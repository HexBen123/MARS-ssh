use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, Context, Result};

const LOG_FILE_NAME: &str = "mars-relay.log";
const SERVICE_NAME: &str = "mars-relay";
const SERVICE_DISPLAY_NAME: &str = "MARS Reverse SSH Tunnel Relay";
const SERVICE_DESCRIPTION: &str = "MARS (Minimal AI Reverse Ssh) 公网中转";

const USAGE: &str = r#"MARS relay —— 公网侧反向隧道中转

用法：
  relay [-config <路径>]              启动（首次运行进入交互向导）
  relay run [-config <路径>]          同上
  relay ms [-config <路径>]           打开服务管理菜单
  relay install [-config <路径>]      注册为系统服务并启动
  relay uninstall                     停止服务并移除注册
"#;

#[cfg(windows)]
windows_service::define_windows_service!(ffi_service_main, relay_service_main);

#[tokio::main]
async fn main() {
    if let Err(err) = real_main().await {
        mars_agent_rs::mars_log!("relay 失败：{err:#}");
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
            ensure_relay_config(&cfg_path, false).await?;
            run_relay_foreground(cfg_path).await
        }
        "ms" | "menu" => {
            let cfg_path = absolute_config_path(&parse_config_path(&args)?)?;
            ensure_relay_config(&cfg_path, false).await?;
            run_relay_menu(cfg_path).await
        }
        "install" => {
            let cfg_path = absolute_config_path(&parse_config_path(&args)?)?;
            ensure_relay_config(&cfg_path, false).await?;
            install_relay_service(cfg_path)
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
fn relay_service_main(_arguments: Vec<std::ffi::OsString>) {
    if let Err(err) = relay_service_entry() {
        mars_agent_rs::mars_log!("relay service failed: {err:#}");
    }
}

#[cfg(windows)]
fn relay_service_entry() -> Result<()> {
    let args = std::env::args().skip(2).collect::<Vec<_>>();
    let cfg_path = parse_config_path(&args)?;
    mars_agent_rs::service::run_windows_service(SERVICE_NAME, |shutdown| async move {
        run_relay_until(cfg_path, async {
            let _ = shutdown.await;
        })
        .await
    })
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

async fn ensure_relay_config(cfg_path: &str, running_as_service: bool) -> Result<()> {
    if mars_agent_rs::setup::should_run_interactive_wizard(
        Path::new(cfg_path).exists(),
        running_as_service,
    ) {
        run_relay_wizard(cfg_path, None).await?;
    }
    Ok(())
}

async fn run_relay_foreground(cfg_path: String) -> Result<()> {
    run_relay_until(cfg_path, wait_for_shutdown()).await
}

async fn run_relay_until<S>(cfg_path: String, shutdown: S) -> Result<()>
where
    S: std::future::Future<Output = ()>,
{
    if let Err(err) = mars_agent_rs::logsink::setup(&cfg_path, LOG_FILE_NAME) {
        mars_agent_rs::mars_log!("提示：无法打开日志文件（{err:#}），仅输出到 stderr");
    }
    let cfg = mars_agent_rs::config::load_relay_config(&cfg_path)?;
    let store = mars_agent_rs::state::Store::load(&cfg.state_file)?;
    let server = mars_agent_rs::relay::Server::new(cfg, store);
    server.run_until_shutdown(shutdown).await
}

async fn run_relay_menu(cfg_path: String) -> Result<()> {
    let bin = std::env::current_exe()
        .context("定位自身可执行文件失败")?
        .to_string_lossy()
        .to_string();
    let install_spec = relay_service_spec(bin, cfg_path.clone());
    let edit_path = cfg_path.clone();
    let run_path = cfg_path.clone();
    mars_agent_rs::menu::run(mars_agent_rs::menu::Spec {
        title: "MARS 中转（relay）".to_string(),
        service_name: SERVICE_NAME.to_string(),
        config_path: cfg_path,
        install_spec,
        edit_config: Box::new(move || {
            let path = edit_path.clone();
            Box::pin(async move {
                let cur = mars_agent_rs::config::load_relay_config(&path)?;
                run_relay_wizard(&path, Some(&cur)).await
            })
        }),
        run_foreground: Some(Box::new(move || {
            let path = run_path.clone();
            Box::pin(async move { run_relay_foreground(path).await })
        })),
    })
    .await
}

fn install_relay_service(cfg_path: String) -> Result<()> {
    let bin = std::env::current_exe()
        .context("定位自身可执行文件失败")?
        .to_string_lossy()
        .to_string();
    mars_agent_rs::config::load_relay_config(&cfg_path)?;
    mars_agent_rs::service::install(relay_service_spec(bin, cfg_path.clone()))?;
    println!("服务 {SERVICE_NAME:?} 已注册并启动（配置：{cfg_path}）");
    println!("现在可以在任何地方直接敲 `ms` 打开管理菜单。");
    Ok(())
}

fn relay_service_spec(bin: String, cfg_path: String) -> mars_agent_rs::service::Spec {
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

async fn run_relay_wizard(
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
    let mut discovered_host = String::new();
    if existing.is_none() {
        print!("正在探测公网 IP ...  ");
        let _ = std::io::stdout().flush();
        match mars_agent_rs::pubip::discover().await {
            Ok(ip) => {
                println!("{ip}");
                discovered_host = ip;
            }
            Err(err) => {
                println!("失败（{err:#}）");
            }
        }
    }
    let default_host = existing
        .map(|cfg| cfg.public_host.as_str())
        .unwrap_or(discovered_host.as_str());
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
