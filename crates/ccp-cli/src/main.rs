//! ccp.exe — com0com-pro 命令行工具。
//!
//! 子命令：init / validate / list / pair / serve（P3）。
//! 服务安装（P4）见 ccp-service。

use ccp_core::{PairManager, SessionConfig};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "ccp",
    about = "com0com-pro：虚拟串口套件控制工具",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 校验 bundles.json 配置
    Validate {
        /// 配置文件路径
        config: PathBuf,
    },
    /// 生成一份示例配置
    Init {
        /// 输出路径（默认 ./bundles.json）
        #[arg(default_value = "bundles.json")]
        output: PathBuf,
    },
    /// 解析 setupc list 输出（从 stdin 读入或调本机 setupc.exe）
    List {
        /// setupc.exe 路径（缺省从常见路径查找）
        #[arg(long)]
        setupc: Option<PathBuf>,
    },
    /// 管理本机 com0com 端口对（经 setupc.exe）
    Pair {
        #[command(subcommand)]
        op: PairOp,
    },
    /// 实机运行配置中的全部 bundle（Ctrl+C 停止）
    Serve {
        /// 配置文件路径
        config: PathBuf,
        /// 活动日志输出路径（缺省 ccp-activity.jsonl）
        #[arg(long, default_value = "ccp-activity.jsonl")]
        log: PathBuf,
    },
    /// 枚举本机串口
    Ports,
    /// 管理 ccp-service Windows 服务（sc.exe）
    Service {
        #[command(subcommand)]
        op: ServiceOp,
    },
}

#[derive(Subcommand)]
enum ServiceOp {
    /// 安装并启动服务（需管理员）
    Install {
        /// ccp-service.exe 路径（缺省为 ccp.exe 同目录）
        #[arg(long)]
        exe: Option<PathBuf>,
        /// 服务自动启动（缺省 true）
        #[arg(long, default_value_t = true)]
        auto: bool,
    },
    /// 停止并删除服务（需管理员）
    Uninstall,
    /// 查询服务状态
    Status,
}

#[derive(Subcommand)]
enum PairOp {
    /// 列出端口对
    List,
    /// 安装端口对：`ccp pair install 0`（编号 "-" 自动分配）
    Install {
        /// 端口对编号（"-" 自动分配）
        id: String,
    },
    /// 删除端口对
    Remove {
        /// 端口对编号
        index: String,
    },
    /// 修改端口参数（如 `ccp pair change CNCA0 EmuBR=yes`）
    Change {
        /// 端口名（CNCA0/CNCB0/COM…）
        port: String,
        /// 参数（如 EmuBR=yes）
        param: String,
    },
    /// 幂等确保端口对存在（缺失则安装）
    Ensure {
        /// 端口对编号
        index: String,
        /// A 侧参数（可多次）
        #[arg(long)]
        a: Vec<String>,
        /// B 侧参数（可多次）
        #[arg(long)]
        b: Vec<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Command::Validate { config } => match SessionConfig::load(&config) {
            Ok(cfg) => {
                for b in &cfg.bundles {
                    println!(
                        "OK  {:<16} {:?}  inlets={:?}  outlets={:?}  policy={}",
                        b.name,
                        b.kind,
                        b.inlets.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
                        b.outlets.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(),
                        b.effective_policy(),
                    );
                }
                println!("config valid: {} bundle(s)", cfg.bundles.len());
            }
            Err(e) => {
                eprintln!("invalid config: {e}");
                std::process::exit(2);
            }
        },
        Command::Init { output } => {
            let cfg = sample_config();
            match cfg.save(&output) {
                Ok(()) => println!("wrote {}", output.display()),
                Err(e) => {
                    eprintln!("failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        Command::List { setupc } => match run_setupc_list(setupc.as_ref()) {
            Ok(text) => match ccp_core::parse_pairs_output(&text) {
                Ok(pairs) => {
                    for p in &pairs {
                        println!(
                            "{} <-> {}   A[{}] B[{}]",
                            p.cnca,
                            p.cncb,
                            p.cnca_params,
                            p.cncb_params
                        );
                    }
                    println!("{} pair(s)", pairs.len());
                }
                Err(e) => {
                    eprintln!("parse error: {e}");
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        Command::Ports => match ccp_core::list_serial_ports() {
            Ok(ports) => {
                if ports.is_empty() {
                    println!("(no serial ports)");
                }
                for p in &ports {
                    println!("{p}");
                }
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        Command::Pair { op } => {
            let Some(pm) = setupc_manager(None) else {
                eprintln!("setupc.exe not found; install com0com or check PATH");
                std::process::exit(1);
            };
            let res = match &op {
                PairOp::List => pm.list().map(|pairs| {
                    for p in &pairs {
                        println!(
                            "{} <-> {}   A[{}] B[{}]",
                            p.cnca, p.cncb, p.cnca_params, p.cncb_params
                        );
                    }
                    println!("{} pair(s)", pairs.len());
                }),
                PairOp::Install { id } => pm.install(id, "-", "-").map(|o| print!("{o}")),
                PairOp::Remove { index } => pm.remove(index).map(|o| print!("{o}")),
                PairOp::Change { port, param } => pm.change(port, param).map(|o| print!("{o}")),
                PairOp::Ensure { index, a, b } => {
                    let a_ref: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
                    let b_ref: Vec<&str> = b.iter().map(|s| s.as_str()).collect();
                    pm.ensure_pair(index, Some(&a_ref.join(",")), Some(&b_ref.join(",")))
                        .map(|created| {
                            println!(
                                "{}",
                                if created {
                                    "created"
                                } else {
                                    "already exists"
                                }
                            )
                        })
                }
            };
            if let Err(e) = res {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Command::Service { op } => {
            if let Err(e) = handle_service(&op) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Command::Serve { config, log } => {
            let cfg = match SessionConfig::load(&config) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("invalid config: {e}");
                    std::process::exit(2);
                }
            };
            let shared_log = match ccp_core::shared_log_to(&log) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("cannot open activity log {}: {e}", log.display());
                    std::process::exit(1);
                }
            };
            let stop = ccp_core::StopFlag::default();
            ctrlc::set_handler({
                let stop = stop.clone();
                move || stop.stop()
            })
            .expect("install Ctrl+C handler");
            println!(
                "serving {} bundle(s); Ctrl+C to stop; activity log: {}",
                cfg.bundles.len(),
                log.display()
            );
            let statuses = ccp_core::run_session(&cfg, &shared_log, &stop);
            for s in &statuses {
                println!(
                    "[{}] {} ({})",
                    if s.ok { "OK" } else { "FAIL" },
                    s.name,
                    s.detail
                );
            }
        }
    }
}

fn setupc_manager(explicit: Option<&PathBuf>) -> Option<PairManager> {
    match explicit {
        Some(p) => Some(PairManager::new(p)),
        None => PairManager::discover(),
    }
}

fn handle_service(op: &ServiceOp) -> Result<(), String> {
    use std::process::Command;
    const NAME: &str = "ccp-service";
    let sc = |args: &[&str]| -> Result<String, String> {
        let out = Command::new("sc.exe")
            .args(args)
            .output()
            .map_err(|e| format!("run sc.exe: {e}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if !out.status.success() {
            return Err(format!("sc {} failed:\n{text}", args.join(" ")));
        }
        Ok(text)
    };
    match op {
        ServiceOp::Install { exe, auto } => {
            let exe = match exe {
                Some(p) => p.clone(),
                None => {
                    let mut p = std::env::current_exe().map_err(|e| e.to_string())?;
                    p.set_file_name("ccp-service.exe");
                    p
                }
            };
            if !exe.exists() {
                return Err(format!("{} not found (build ccp-service first)", exe.display()));
            }
            let bin = format!("\"{}\" --service", exe.display());
            let start = if *auto { "auto" } else { "demand" };
            sc(&[
                "create", NAME, "binPath=", &bin, "start=", start,
                "DisplayName=", "com0com-pro Bundle Service",
            ])?;
            let text = sc(&["start", NAME])?;
            print!("{text}");
            println!("service `{NAME}` installed and started");
            Ok(())
        }
        ServiceOp::Uninstall => {
            let _ = sc(&["stop", NAME]);
            let text = sc(&["delete", NAME])?;
            print!("{text}");
            println!("service `{NAME}` removed");
            Ok(())
        }
        ServiceOp::Status => {
            let text = sc(&["query", NAME])?;
            print!("{text}");
            Ok(())
        }
    }
}

fn sample_config() -> SessionConfig {
    use ccp_core::{AclRule, BundleKind, BundleSpec, EndpointKind, EndpointSpec};
    let port = |name: &str, port: &str| EndpointSpec {
        name: name.into(),
        kind: EndpointKind::Port { port: port.into() },
    };
    SessionConfig {
        bundles: vec![
            BundleSpec {
                name: "gps-split".into(),
                kind: BundleKind::Split,
                inlets: vec![port("gps", "COM3")],
                outlets: vec![port("nav", "CNCA0"), port("log", "CNCA1")],
                policy: None,
                acl: vec![AclRule {
                    process: "*".into(),
                    access: "read".into(),
                }],
            },
            BundleSpec {
                name: "tool-link".into(),
                kind: BundleKind::Redirect,
                inlets: vec![port("side-a", "CNCA2")],
                outlets: vec![port("side-b", "CNCA3")],
                policy: Some("one-to-one".into()),
                acl: vec![],
            },
        ],
    }
}

fn run_setupc_list(setupc: Option<&PathBuf>) -> Result<String, String> {
    use std::process::Command;
    let exe: PathBuf = setupc
        .cloned()
        .or_else(which_setupc)
        .ok_or_else(|| "setupc.exe not found; install com0com or pass --setupc".to_string())?;
    let out = Command::new(&exe)
        .arg("list")
        .output()
        .map_err(|e| format!("failed to run {}: {e}", exe.display()))?;
    if !out.status.success() {
        return Err(format!(
            "{} list exited with {}",
            exe.display(),
            out.status
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn which_setupc() -> Option<PathBuf> {
    // 常见安装路径
    let candidates = [
        r"C:\Program Files (x86)\com0com\setupc.exe",
        r"C:\Program Files\com0com\setupc.exe",
    ];
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}
