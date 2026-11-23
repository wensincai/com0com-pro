//! ccp-service — com0com-pro Windows 服务宿主。
//!
//! 职责（docs/04-架构设计.md §4）：开机（登录前）按 bundles.json 重建全部
//! bundle，直到 SCM 下发 Stop。
//!
//! 用法：
//! - SCM 入口：`ccp-service --service`（由服务控制管理器调用）
//! - 前台调试：`ccp-service --console`
//! - 安装/卸载：`ccp service install|uninstall`（在 ccp-cli 中，调 sc.exe）

use ccp_core::{run_session, shared_log_to, SessionConfig, StopFlag};
use std::path::PathBuf;
use std::time::Duration;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;

const SERVICE_NAME: &str = "ccp-service";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

windows_service::define_windows_service!(ffi_service_main, service_main);

fn service_main(_args: Vec<std::ffi::OsString>) {
    if let Err(e) = run_service() {
        eprintln!("service failed: {e}");
    }
}

/// 默认配置路径：`%ProgramData%\com0com-pro\bundles.json`。
pub fn default_config_path() -> PathBuf {
    base_dir().join("bundles.json")
}

fn default_log_path() -> PathBuf {
    base_dir().join("activity.jsonl")
}

fn base_dir() -> PathBuf {
    let program_data = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into());
    PathBuf::from(program_data).join("com0com-pro")
}

fn run_service() -> windows_service::Result<()> {
    let stop = StopFlag::default();
    let stop_for_handler = stop.clone();

    let event_handler = move |control: ServiceControl| -> ServiceControlHandlerResult {
        match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                stop_for_handler.stop();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    let set = |state: ServiceState, accept: ServiceControlAccept, checkpoint: u32| {
        status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        })
    };

    set(ServiceState::StartPending, ServiceControlAccept::empty(), 1)?;
    set(ServiceState::Running, ServiceControlAccept::STOP, 0)?;

    let config_path = default_config_path();
    let log_path = default_log_path();
    match SessionConfig::load(&config_path) {
        Ok(cfg) => {
            if cfg.bundles.is_empty() {
                eprintln!(
                    "config {} has no bundles; nothing to serve",
                    config_path.display()
                );
            }
            let _ = std::fs::create_dir_all(&log_path);
            match shared_log_to(&log_path) {
                Ok(log) => {
                    let statuses = run_session(&cfg, &log, &stop);
                    for s in &statuses {
                        eprintln!(
                            "[{}] {} ({})",
                            if s.ok { "OK" } else { "FAIL" },
                            s.name,
                            s.detail
                        );
                    }
                }
                Err(e) => eprintln!("activity log open failed ({}): {e}", log_path.display()),
            }
        }
        Err(e) => eprintln!("config load failed ({}): {e}", config_path.display()),
    }

    set(ServiceState::StopPending, ServiceControlAccept::empty(), 2)?;
    set(ServiceState::Stopped, ServiceControlAccept::empty(), 0)?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--service") {
        if let Err(e) = service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
            eprintln!("dispatcher error (not started by SCM?): {e}");
            std::process::exit(1);
        }
        return;
    }

    // 前台调试模式
    let config = default_config_path();
    let log = default_log_path();
    let cfg = match SessionConfig::load(&config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config load failed ({}): {e}", config.display());
            eprintln!("tip: `ccp init bundles.json` then copy it to the path above");
            std::process::exit(2);
        }
    };
    let logf = match shared_log_to(&log) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("log open failed ({}): {e}", log.display());
            std::process::exit(1);
        }
    };
    let stop = StopFlag::default();
    let stop2 = stop.clone();
    let _ = ctrlc::set_handler(move || stop2.stop());
    println!(
        "console mode: {} bundle(s), config {}, log {} (Ctrl+C to stop)",
        cfg.bundles.len(),
        config.display(),
        log.display()
    );
    let statuses = run_session(&cfg, &logf, &stop);
    for s in &statuses {
        println!("[{}] {} ({})", if s.ok { "OK" } else { "FAIL" }, s.name, s.detail);
    }
}
