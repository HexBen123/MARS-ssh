use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Spec {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub bin_path: String,
    pub config_path: String,
    pub args: Vec<String>,
    pub user: String,
    pub group: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub installed: bool,
    pub running: bool,
    pub enabled: bool,
    pub detail: String,
}

pub fn render_systemd_unit(spec: &Spec) -> String {
    let mut out = String::new();
    out.push_str("[Unit]\n");
    out.push_str(&format!(
        "Description={}\nAfter=network-online.target\nWants=network-online.target\n\n",
        first_non_empty(&[&spec.description, &spec.display_name, &spec.name])
    ));
    out.push_str("[Service]\nType=simple\n");
    out.push_str("ExecStart=");
    out.push_str(&systemd_quote(&spec.bin_path));
    for arg in &spec.args {
        out.push(' ');
        out.push_str(&systemd_quote(arg));
    }
    out.push_str("\nRestart=always\nRestartSec=3\n");
    if !spec.user.is_empty() {
        out.push_str(&format!("User={}\n", spec.user));
    }
    if !spec.group.is_empty() {
        out.push_str(&format!("Group={}\n", spec.group));
    }
    out.push_str("\n[Install]\nWantedBy=multi-user.target\n");
    out
}

pub fn extract_exe_path(command_line: &str) -> Option<String> {
    let command_line = command_line.trim();
    if command_line.is_empty() {
        return None;
    }
    if let Some(rest) = command_line.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    command_line
        .split_once(' ')
        .map(|(exe, _)| exe.to_string())
        .or_else(|| Some(command_line.to_string()))
}

pub fn install(spec: Spec) -> Result<()> {
    platform::install(spec)
}

pub fn uninstall(name: &str) -> Result<()> {
    platform::uninstall(name)
}

pub fn query_status(name: &str) -> Result<Status> {
    platform::query_status(name)
}

pub fn start(name: &str) -> Result<()> {
    platform::start(name)
}

pub fn stop(name: &str) -> Result<()> {
    platform::stop(name)
}

pub fn restart(name: &str) -> Result<()> {
    platform::restart(name)
}

pub fn enable(name: &str) -> Result<()> {
    platform::enable(name)
}

pub fn disable(name: &str) -> Result<()> {
    platform::disable(name)
}

#[cfg(windows)]
pub fn try_start_dispatcher(
    service_name: &str,
    service_main: extern "system" fn(u32, *mut *mut u16),
) -> Result<bool> {
    const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: i32 = 1063;
    match windows_service::service_dispatcher::start(service_name, service_main) {
        Ok(()) => Ok(true),
        Err(windows_service::Error::Winapi(err))
            if err.raw_os_error() == Some(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT) =>
        {
            Ok(false)
        }
        Err(err) => Err(anyhow::anyhow!("start service dispatcher: {err}")),
    }
}

#[cfg(not(windows))]
pub fn try_start_dispatcher(
    _service_name: &str,
    _service_main: extern "system" fn(u32, *mut *mut u16),
) -> Result<bool> {
    Ok(false)
}

#[cfg(windows)]
pub fn run_windows_service<F, Fut>(service_name: &str, run: F) -> Result<()>
where
    F: FnOnce(tokio::sync::oneshot::Receiver<()>) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    use std::time::Duration;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let mut shutdown_tx = Some(shutdown_tx);
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            ServiceControl::Stop => {
                if let Some(tx) = shutdown_tx.take() {
                    let _ = tx.send(());
                }
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(service_name, event_handler)
        .context("register service control handler")?;
    status_handle
        .set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
        .context("set service running status")?;

    let runtime = tokio::runtime::Runtime::new().context("create service runtime")?;
    let result = runtime.block_on(run(shutdown_rx));

    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });
    result
}

fn first_non_empty<'a>(values: &[&'a str]) -> &'a str {
    values
        .iter()
        .copied()
        .find(|value| !value.is_empty())
        .unwrap_or("")
}

fn systemd_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn ms_shortcut_for_windows(spec: &Spec) -> String {
    format!(
        "@echo off\r\nREM mars-shortcut-for={}\r\n\"{}\" ms -config \"{}\" %*\r\n",
        spec.name, spec.bin_path, spec.config_path
    )
}

#[cfg(target_os = "linux")]
fn ms_shortcut_for_unix(spec: &Spec) -> String {
    format!(
        "#!/bin/sh\n# mars-shortcut-for={}\nexec \"{}\" ms -config \"{}\" \"$@\"\n",
        spec.name, spec.bin_path, spec.config_path
    )
}

fn write_ms_shortcut_windows(spec: &Spec) -> Result<()> {
    let path = Path::new(&spec.bin_path)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("ms.cmd");
    std::fs::write(&path, ms_shortcut_for_windows(spec))
        .with_context(|| format!("write {}", path.display()))
}

#[cfg(target_os = "linux")]
fn write_ms_shortcut_unix(spec: &Spec) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = Path::new("/usr/local/bin/ms");
    std::fs::write(path, ms_shortcut_for_unix(spec))
        .with_context(|| format!("write {}", path.display()))?;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn run_command(name: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(name)
        .args(args)
        .output()
        .with_context(|| format!("run {name} {}", args.join(" ")))?;
    if !output.status.success() {
        anyhow::bail!(
            "{} {} failed: {}\n{}",
            name,
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    const UNIT_DIR: &str = "/etc/systemd/system";

    pub fn install(spec: Spec) -> Result<()> {
        let unit_path = PathBuf::from(UNIT_DIR).join(format!("{}.service", spec.name));
        std::fs::write(&unit_path, render_systemd_unit(&spec))
            .with_context(|| format!("write {} (are you root?)", unit_path.display()))?;
        run_command("systemctl", &["daemon-reload"])?;
        run_command("systemctl", &["enable", "--now", &spec.name])?;
        let _ = write_ms_shortcut_unix(&spec);
        Ok(())
    }

    pub fn uninstall(name: &str) -> Result<()> {
        let _ = run_command("systemctl", &["stop", name]);
        let _ = run_command("systemctl", &["disable", name]);
        let unit_path = PathBuf::from(UNIT_DIR).join(format!("{name}.service"));
        if let Err(err) = std::fs::remove_file(&unit_path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(err).with_context(|| format!("remove {}", unit_path.display()));
            }
        }
        let _ = run_command("systemctl", &["daemon-reload"]);
        remove_unix_ms_shortcut_if_owned(name);
        Ok(())
    }

    pub fn query_status(name: &str) -> Result<Status> {
        let unit_path = PathBuf::from(UNIT_DIR).join(format!("{name}.service"));
        let installed = unit_path.exists();
        let active = probe("systemctl", &["is-active", name]);
        let enabled = probe("systemctl", &["is-enabled", name]);
        let detail = probe(
            "systemctl",
            &[
                "show",
                name,
                "--property=ActiveState,SubState,MainPID",
                "--value",
            ],
        )
        .replace('\n', " ");
        Ok(Status {
            installed,
            running: matches!(active.as_str(), "active" | "activating"),
            enabled: matches!(enabled.as_str(), "enabled" | "alias" | "static"),
            detail,
        })
    }

    pub fn start(name: &str) -> Result<()> {
        run_command("systemctl", &["start", name]).map(|_| ())
    }
    pub fn stop(name: &str) -> Result<()> {
        run_command("systemctl", &["stop", name]).map(|_| ())
    }
    pub fn restart(name: &str) -> Result<()> {
        run_command("systemctl", &["restart", name]).map(|_| ())
    }
    pub fn enable(name: &str) -> Result<()> {
        run_command("systemctl", &["enable", name]).map(|_| ())
    }
    pub fn disable(name: &str) -> Result<()> {
        run_command("systemctl", &["disable", name]).map(|_| ())
    }

    fn probe(name: &str, args: &[&str]) -> String {
        Command::new(name)
            .args(args)
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_default()
    }

    fn remove_unix_ms_shortcut_if_owned(name: &str) {
        let path = Path::new("/usr/local/bin/ms");
        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };
        if content.contains(&format!("mars-shortcut-for={name}")) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::OsString;
    use std::time::{Duration, Instant};
    use windows_service::service::{
        ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState,
        ServiceType,
    };
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    pub fn install(spec: Spec) -> Result<()> {
        let manager =
            ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)
                .context("connect SCM (run as Administrator)")?;
        if manager
            .open_service(&spec.name, ServiceAccess::QUERY_STATUS)
            .is_ok()
        {
            anyhow::bail!(
                "service {:?} already exists; run `uninstall` first",
                spec.name
            );
        }
        let info = ServiceInfo {
            name: OsString::from(&spec.name),
            display_name: OsString::from(first_non_empty(&[&spec.display_name, &spec.name])),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: PathBuf::from(&spec.bin_path),
            launch_arguments: spec.args.iter().map(OsString::from).collect(),
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };
        let service = manager
            .create_service(
                &info,
                ServiceAccess::START | ServiceAccess::QUERY_STATUS | ServiceAccess::CHANGE_CONFIG,
            )
            .context("create service")?;
        service.start::<OsString>(&[]).context("start service")?;
        let _ = configure_failure_actions(&spec.name);
        let _ = write_ms_shortcut_windows(&spec);
        Ok(())
    }

    pub fn uninstall(name: &str) -> Result<()> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .context("connect SCM (run as Administrator)")?;
        let service = match manager.open_service(
            name,
            ServiceAccess::QUERY_STATUS
                | ServiceAccess::STOP
                | ServiceAccess::DELETE
                | ServiceAccess::QUERY_CONFIG,
        ) {
            Ok(service) => service,
            Err(_) => return Ok(()),
        };
        if let Ok(status) = service.query_status() {
            if status.current_state != ServiceState::Stopped {
                let _ = service.stop();
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(10) {
                    if service
                        .query_status()
                        .map(|status| status.current_state == ServiceState::Stopped)
                        .unwrap_or(true)
                    {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
        let bin_path = service
            .query_config()
            .ok()
            .map(|config| config.executable_path.to_string_lossy().to_string());
        service.delete().context("delete service")?;
        if let Some(bin_path) = bin_path {
            remove_windows_ms_shortcut_if_owned(&bin_path, name);
        }
        Ok(())
    }

    pub fn query_status(name: &str) -> Result<Status> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .context("connect SCM")?;
        let service = match manager.open_service(
            name,
            ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG,
        ) {
            Ok(service) => service,
            Err(_) => return Ok(Status::default()),
        };
        let status = service.query_status().context("query service status")?;
        let running = matches!(
            status.current_state,
            ServiceState::Running | ServiceState::StartPending
        );
        let enabled = service
            .query_config()
            .map(|config| config.start_type == ServiceStartType::AutoStart)
            .unwrap_or(false);
        Ok(Status {
            installed: true,
            running,
            enabled,
            detail: service_state_detail(status.current_state).to_string(),
        })
    }

    pub fn start(name: &str) -> Result<()> {
        with_service(name, ServiceAccess::START, |service| {
            service.start::<OsString>(&[]).map_err(Into::into)
        })
    }
    pub fn stop(name: &str) -> Result<()> {
        with_service(
            name,
            ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
            |service| {
                if service
                    .query_status()
                    .map(|status| status.current_state == ServiceState::Stopped)
                    .unwrap_or(false)
                {
                    return Ok(());
                }
                service.stop().map(|_| ()).map_err(anyhow::Error::from)?;
                wait_for_stopped(&service, name, Duration::from_secs(10))
            },
        )
    }
    pub fn restart(name: &str) -> Result<()> {
        stop(name)?;
        start(name)
    }
    pub fn enable(name: &str) -> Result<()> {
        run_sc_config(name, "auto")
    }
    pub fn disable(name: &str) -> Result<()> {
        run_sc_config(name, "demand")
    }

    fn with_service<F>(name: &str, access: ServiceAccess, action: F) -> Result<()>
    where
        F: FnOnce(windows_service::service::Service) -> Result<()>,
    {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .context("connect SCM (run as Administrator)")?;
        let service = manager.open_service(name, access).context("open service")?;
        action(service)
    }

    fn wait_for_stopped(
        service: &windows_service::service::Service,
        name: &str,
        timeout: Duration,
    ) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if service
                .query_status()
                .map(|status| status.current_state == ServiceState::Stopped)
                .unwrap_or(true)
            {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        anyhow::bail!("timeout waiting for {name:?} to stop")
    }

    fn service_state_detail(state: ServiceState) -> &'static str {
        match state {
            ServiceState::Running | ServiceState::StartPending => "running",
            ServiceState::StopPending => "stopping",
            ServiceState::PausePending | ServiceState::Paused | ServiceState::ContinuePending => {
                "paused"
            }
            _ => "stopped",
        }
    }

    fn configure_failure_actions(name: &str) -> Result<()> {
        let args = windows_failure_recovery_args(name);
        let status = Command::new("sc.exe")
            .args(args)
            .status()
            .context("run sc.exe failure")?;
        if !status.success() {
            anyhow::bail!("sc.exe failure {name} failed: {status}");
        }
        Ok(())
    }

    fn windows_failure_recovery_args(name: &str) -> [&str; 6] {
        [
            "failure",
            name,
            "reset=",
            "86400",
            "actions=",
            "restart/3000/restart/3000/restart/5000",
        ]
    }

    fn run_sc_config(name: &str, start_type: &str) -> Result<()> {
        let status = Command::new("sc.exe")
            .args(["config", name, "start=", start_type])
            .status()
            .context("run sc.exe config")?;
        if !status.success() {
            anyhow::bail!("sc.exe config {name} failed: {status}");
        }
        Ok(())
    }

    fn remove_windows_ms_shortcut_if_owned(bin_path: &str, name: &str) {
        let path = Path::new(bin_path)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("ms.cmd");
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        if content.contains(&format!("mars-shortcut-for={name}")) {
            let _ = std::fs::remove_file(path);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn renders_windows_failure_recovery_sc_arguments() {
            assert_eq!(
                windows_failure_recovery_args("mars-agent"),
                [
                    "failure",
                    "mars-agent",
                    "reset=",
                    "86400",
                    "actions=",
                    "restart/3000/restart/3000/restart/5000",
                ]
            );
        }

        #[test]
        fn maps_service_states_to_go_style_status_detail() {
            assert_eq!(service_state_detail(ServiceState::Running), "running");
            assert_eq!(service_state_detail(ServiceState::StopPending), "stopping");
            assert_eq!(service_state_detail(ServiceState::Paused), "paused");
            assert_eq!(service_state_detail(ServiceState::Stopped), "stopped");
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::*;

    pub fn install(_spec: Spec) -> Result<()> {
        anyhow::bail!("service install not supported on this OS")
    }
    pub fn uninstall(_name: &str) -> Result<()> {
        anyhow::bail!("service uninstall not supported on this OS")
    }
    pub fn query_status(_name: &str) -> Result<Status> {
        Ok(Status::default())
    }
    pub fn start(_name: &str) -> Result<()> {
        anyhow::bail!("not supported on this OS")
    }
    pub fn stop(_name: &str) -> Result<()> {
        anyhow::bail!("not supported on this OS")
    }
    pub fn restart(_name: &str) -> Result<()> {
        anyhow::bail!("not supported on this OS")
    }
    pub fn enable(_name: &str) -> Result<()> {
        anyhow::bail!("not supported on this OS")
    }
    pub fn disable(_name: &str) -> Result<()> {
        anyhow::bail!("not supported on this OS")
    }
}
