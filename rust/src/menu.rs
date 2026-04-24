use std::io::Write;
use std::pin::Pin;

use anyhow::{Context, Result};

use crate::service;

pub type MenuFuture = Pin<Box<dyn std::future::Future<Output = Result<()>>>>;

pub struct Spec {
    pub title: String,
    pub service_name: String,
    pub config_path: String,
    pub install_spec: service::Spec,
    pub edit_config: Box<dyn Fn() -> MenuFuture>,
    pub run_foreground: Option<Box<dyn Fn() -> MenuFuture>>,
}

pub fn redact_config_text(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if let Some(value) = trimmed.strip_prefix("token:") {
                let prefix_len = line.len() - trimmed.len();
                let prefix = &line[..prefix_len];
                let value = value.trim();
                if value.len() > 12 {
                    format!("{prefix}token: {}...（已省略）", &value[..8])
                } else {
                    line.to_string()
                }
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn run(spec: Spec) -> Result<()> {
    loop {
        let status = service::query_status(&spec.service_name).unwrap_or_default();
        render_header(&spec, &status);
        let options = if status.installed {
            installed_options(spec.run_foreground.is_some())
        } else {
            not_installed_options(spec.run_foreground.is_some())
        };
        for (key, label) in &options {
            println!(" {key}) {label}");
        }
        println!(" q) 退出");
        println!("-----------------------------------------------------");
        print!("选择： ");
        std::io::stdout().flush().context("flush stdout")?;

        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .context("read stdin")?;
        let choice = line.trim();
        if choice.eq_ignore_ascii_case("q") || choice.is_empty() {
            return Ok(());
        }

        let result = match choice {
            "1" => view_config(&spec.config_path),
            "2" => (spec.edit_config)().await,
            "3" if status.installed => service::start(&spec.service_name),
            "3" => service::install(spec.install_spec.clone()),
            "4" if status.installed => service::stop(&spec.service_name),
            "4" => {
                if let Some(run_foreground) = &spec.run_foreground {
                    run_foreground().await
                } else {
                    Ok(())
                }
            }
            "5" if status.installed => service::restart(&spec.service_name),
            "6" if status.installed => service::enable(&spec.service_name),
            "7" if status.installed => service::disable(&spec.service_name),
            "8" if status.installed => service::uninstall(&spec.service_name),
            _ => {
                println!("\n!! 无效选项：{choice:?}");
                Ok(())
            }
        };

        if let Err(err) = result {
            println!("\n!! 操作失败：{err:#}");
        }
        println!();
        print!("按回车继续 ...");
        let _ = std::io::stdout().flush();
        let mut pause = String::new();
        let _ = std::io::stdin().read_line(&mut pause);
    }
}

fn installed_options(_has_foreground: bool) -> Vec<(&'static str, &'static str)> {
    vec![
        ("1", "查看当前配置"),
        ("2", "修改配置（保存后需重启服务生效）"),
        ("3", "启动服务"),
        ("4", "停止服务"),
        ("5", "重启服务"),
        ("6", "设为开机自启"),
        ("7", "取消开机自启"),
        ("8", "卸载服务"),
    ]
}

fn not_installed_options(has_foreground: bool) -> Vec<(&'static str, &'static str)> {
    let mut options = vec![
        ("1", "查看当前配置"),
        ("2", "修改配置"),
        ("3", "注册为系统服务并启动"),
    ];
    if has_foreground {
        options.push(("4", "前台运行一次（Ctrl+C 停止）"));
    }
    options
}

fn render_header(spec: &Spec, status: &service::Status) {
    print!("\x1b[2J\x1b[H");
    println!("=====================================================");
    println!(" {}", spec.title);
    println!("=====================================================");
    println!(" 服务名  ： {}", spec.service_name);
    println!(" 配置文件： {}", spec.config_path);
    println!(" 状态    ： {}", render_status(status));
    println!("-----------------------------------------------------");
}

fn render_status(status: &service::Status) -> String {
    if !status.installed {
        return "未安装为系统服务".to_string();
    }
    let mut parts = Vec::new();
    parts.push(if status.running {
        "运行中"
    } else {
        "已停止"
    });
    parts.push(if status.enabled {
        "开机自启"
    } else {
        "未设为自启"
    });
    let mut label = parts.join(" / ");
    if !status.detail.is_empty() {
        label.push_str("  (");
        label.push_str(&status.detail);
        label.push(')');
    }
    label
}

fn view_config(path: &str) -> Result<()> {
    let content = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    println!();
    println!("-----------------------------------------------------");
    println!(" 配置文件：{path}");
    println!("-----------------------------------------------------");
    println!("{}", redact_config_text(&content));
    println!("-----------------------------------------------------");
    Ok(())
}
