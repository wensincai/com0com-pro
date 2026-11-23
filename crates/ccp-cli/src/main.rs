//! ccp.exe — com0com-pro 命令行工具。
//!
//! P2 范围：配置校验、示例生成、setupc 端口对列表解析。
//! P3 追加：pair 管理（调 setupc）、bundle 实机运行（串口/管道泵）。

use ccp_core::{parse_pairs_output, SessionConfig};
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
        /// setupc.exe 路径（缺省从 PATH 查找）
        #[arg(long)]
        setupc: Option<PathBuf>,
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
            Ok(text) => match parse_pairs_output(&text) {
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
