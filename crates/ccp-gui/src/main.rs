//! ccp-gui — com0com-pro 管理界面（egui）。
//!
//! P4 功能：
//! - bundle 列表/编辑（名称、类型、端点表、策略）
//! - 引擎启停（进程内线程，Ctrl+C 语义由 StopFlag 承担）
//! - 活动日志实时视图（tail JSONL）
//! - 端口对管理（setupc.exe）与串口枚举
//!
//! 设计取舍：P4 阶段 GUI 直接进程内跑引擎（单机管理场景足够），
//! 命名管道 JSON-RPC（FR-A6）留待与第三方集成时再拆分进程。

// release 构建隐藏控制台窗口（debug 保留便于排错）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ccp_core::{
    list_serial_ports, run_session, shared_log_to, BundleKind, BundleSpec, EndpointKind,
    EndpointSpec, PairManager, SessionConfig, StopFlag,
};
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([840.0, 520.0])
            .with_title("com0com-pro"),
        ..Default::default()
    };
    eframe::run_native(
        "com0com-pro",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

/// 运行中的引擎句柄。
struct Running {
    log_path: PathBuf,
    stop: StopFlag,
    started: std::time::Instant,
}

struct App {
    config_path: PathBuf,
    cfg: SessionConfig,
    selected: Option<usize>,
    /// 编辑缓冲：端点表以 "name:port" 行文本编辑
    inlets_buf: String,
    outlets_buf: String,
    policy_buf: String,
    status: String,
    running: Option<Running>,
    activity: Vec<String>,
    serial_ports: Vec<String>,
    pairs: Vec<String>,
    setupc_missing: bool,
    auto_refresh: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_cjk_fonts(&cc.egui_ctx);
        let config_path = default_config_path();
        let cfg = SessionConfig::load(&config_path).unwrap_or_default();
        let mut app = Self {
            config_path,
            cfg,
            selected: None,
            inlets_buf: String::new(),
            outlets_buf: String::new(),
            policy_buf: String::new(),
            status: "idle".into(),
            running: None,
            activity: Vec::new(),
            serial_ports: Vec::new(),
            pairs: Vec::new(),
            setupc_missing: PairManager::discover().is_none(),
            auto_refresh: true,
        };
        app.refresh_ports();
        app
    }

    fn refresh_ports(&mut self) {
        self.serial_ports = list_serial_ports().unwrap_or_default();
        self.pairs = match PairManager::discover() {
            Some(pm) => pm
                .list()
                .map(|pairs| {
                    pairs
                        .iter()
                        .map(|p| {
                            format!(
                                "{} <-> {}  A[{}] B[{}]",
                                p.cnca, p.cncb, p.cnca_params, p.cncb_params
                            )
                        })
                        .collect()
                })
                .unwrap_or_else(|e| vec![format!("list error: {e}")]),
            None => Vec::new(),
        };
    }

    fn load_selected_into_buffers(&mut self) {
        if let Some(i) = self.selected {
            if let Some(b) = self.cfg.bundles.get(i) {
                self.inlets_buf = b
                    .inlets
                    .iter()
                    .map(|e| format!("{}:{}", e.name, endpoint_port(e)))
                    .collect::<Vec<_>>()
                    .join("\n");
                self.outlets_buf = b
                    .outlets
                    .iter()
                    .map(|e| format!("{}:{}", e.name, endpoint_port(e)))
                    .collect::<Vec<_>>()
                    .join("\n");
                self.policy_buf = b.policy.clone().unwrap_or_default();
            }
        }
    }

    fn apply_buffers(&mut self) {
        let Some(i) = self.selected else { return };
        let Some(b) = self.cfg.bundles.get_mut(i) else { return };
        b.inlets = parse_endpoint_lines(&self.inlets_buf);
        b.outlets = parse_endpoint_lines(&self.outlets_buf);
        b.policy = if self.policy_buf.trim().is_empty() {
            None
        } else {
            Some(self.policy_buf.trim().to_string())
        };
    }

    fn start_engine(&mut self) {
        self.apply_buffers();
        if let Err(e) = self.cfg.validate() {
            self.status = format!("config invalid: {e}");
            return;
        }
        let log_path = self
            .config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("ccp-activity.jsonl");
        let log = match shared_log_to(&log_path) {
            Ok(l) => l,
            Err(e) => {
                self.status = format!("log open failed: {e}");
                return;
            }
        };
        let stop = StopFlag::default();
        let cfg = self.cfg.clone();
        let count = cfg.bundles.len();
        let stop2 = stop.clone();
        let log2 = Arc::clone(&log);
        std::thread::spawn(move || {
            run_session(&cfg, &log2, &stop2);
        });
        self.status = format!("running {count} bundle(s)");
        self.running = Some(Running {
            log_path,
            stop,
            started: std::time::Instant::now(),
        });
    }

    fn stop_engine(&mut self) {
        if let Some(r) = self.running.take() {
            r.stop.stop();
            self.status = "stopped".into();
        }
    }

    fn refresh_activity(&mut self) {
        let Some(r) = &self.running else { return };
        if let Ok(text) = std::fs::read_to_string(&r.log_path) {
            let lines: Vec<String> = text
                .lines()
                .rev()
                .take(200)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            self.activity = lines;
        }
    }
}

/// 安装中文字体：egui 默认字体不含 CJK 字形（显示为方块）。
/// 从 Windows 系统字体目录加载微软雅黑，追加到 Proportional 与
/// Monospace 两个字族的回退链尾部（拉丁字符仍用 egui 内置字体）。
fn install_cjk_fonts(ctx: &egui::Context) {
    let candidates: [&str; 3] = ["msyh.ttc", "simhei.ttf", "simsun.ttc"];
    let mut loaded: Option<(String, egui::FontData)> = None;
    for name in candidates {
        let path = PathBuf::from(r"C:\Windows\Fonts").join(name);
        if let Ok(bytes) = std::fs::read(&path) {
            loaded = Some((format!("cjk-{name}"), egui::FontData::from_owned(bytes)));
            break;
        }
    }
    let Some((loaded_name, data)) = loaded else {
        // 非 Windows 或缺字体目录：保持默认，英文界面仍可用
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(loaded_name.clone(), data.into());
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(loaded_name.clone());
    }
    ctx.set_fonts(fonts);
}

fn endpoint_port(e: &EndpointSpec) -> String {
    match &e.kind {
        EndpointKind::Port { port } => port.clone(),
        EndpointKind::Tcp { addr } => format!("tcp://{addr}"),
        EndpointKind::Pipe { path } => format!("pipe:{path}"),
    }
}

fn parse_endpoint_lines(text: &str) -> Vec<EndpointSpec> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (name, target) = match line.split_once(':') {
                Some((n, t)) => (n.trim(), t.trim()),
                None => (line, line),
            };
            Some(EndpointSpec {
                name: name.to_string(),
                kind: if let Some(addr) = target.strip_prefix("tcp://") {
                    EndpointKind::Tcp { addr: addr.to_string() }
                } else if let Some(p) = target.strip_prefix("pipe:") {
                    EndpointKind::Pipe { path: p.to_string() }
                } else {
                    EndpointKind::Port { port: target.to_string() }
                },
            })
        })
        .collect()
}

fn default_config_path() -> PathBuf {
    // 优先当前目录 bundles.json；否则 %ProgramData%
    let local = PathBuf::from("bundles.json");
    if local.exists() {
        return local;
    }
    let program_data = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into());
    PathBuf::from(program_data)
        .join("com0com-pro")
        .join("bundles.json")
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 运行中每 500ms 刷新活动日志
        if self.running.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
            self.refresh_activity();
        }

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let running = self.running.is_some();
                if ui
                    .add_enabled(!running, egui::Button::new("▶ 启动引擎"))
                    .clicked()
                {
                    self.start_engine();
                }
                if ui
                    .add_enabled(running, egui::Button::new("■ 停止"))
                    .clicked()
                {
                    self.stop_engine();
                }
                ui.separator();
                if ui.button("加载配置").clicked() {
                    match SessionConfig::load(&self.config_path) {
                        Ok(c) => {
                            self.cfg = c;
                            self.selected = None;
                            self.status = "config loaded".into();
                        }
                        Err(e) => self.status = format!("load failed: {e}"),
                    }
                }
                if ui.button("保存配置").clicked() {
                    self.apply_buffers();
                    match self.cfg.save(&self.config_path) {
                        Ok(()) => self.status = format!("saved {}", self.config_path.display()),
                        Err(e) => self.status = format!("save failed: {e}"),
                    }
                }
                ui.separator();
                if ui.button("刷新端口").clicked() {
                    self.refresh_ports();
                }
                ui.checkbox(&mut self.auto_refresh, "自动刷新");
                if let Some(r) = &self.running {
                    let secs = r.started.elapsed().as_secs();
                    ui.label(
                        egui::RichText::new(format!("● running {secs}s"))
                            .color(egui::Color32::from_rgb(80, 200, 120)),
                    );
                } else {
                    ui.label(egui::RichText::new("○ idle").weak());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&self.status).weak());
                });
            });
        });

        egui::SidePanel::left("bundles")
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.heading("Bundles");
                ui.label(egui::RichText::new(self.config_path.display().to_string()).small().weak());
                ui.separator();
                let rows: Vec<(usize, String)> = self
                    .cfg
                    .bundles
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        (
                            i,
                            format!(
                                "{:<14} {:?}\n{} → {}",
                                b.name,
                                b.kind,
                                b.inlets.len(),
                                b.outlets.len()
                            ),
                        )
                    })
                    .collect();
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for (i, label) in rows {
                        let selected = self.selected == Some(i);
                        if ui.selectable_label(selected, label).clicked() {
                            self.selected = Some(i);
                            self.load_selected_into_buffers();
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("＋ 新增").clicked() {
                        let n = self.cfg.bundles.len();
                        self.cfg.bundles.push(BundleSpec {
                            name: format!("bundle{n}"),
                            kind: BundleKind::Redirect,
                            inlets: vec![EndpointSpec {
                                name: "a".into(),
                                kind: EndpointKind::Port { port: "CNCA0".into() },
                            }],
                            outlets: vec![EndpointSpec {
                                name: "b".into(),
                                kind: EndpointKind::Port { port: "CNCB0".into() },
                            }],
                            policy: None,
                            acl: vec![],
                        });
                        self.selected = Some(n);
                        self.load_selected_into_buffers();
                    }
                    if ui
                        .add_enabled(self.selected.is_some(), egui::Button::new("－ 删除"))
                        .clicked()
                    {
                        if let Some(i) = self.selected {
                            self.cfg.bundles.remove(i);
                            self.selected = None;
                        }
                    }
                });
                ui.separator();
                ui.heading("串口");
                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    if self.serial_ports.is_empty() {
                        ui.label(egui::RichText::new("(无)").weak());
                    }
                    for p in &self.serial_ports {
                        ui.label(p);
                    }
                });
                ui.heading("com0com 端口对");
                if self.setupc_missing {
                    ui.label(
                        egui::RichText::new(
                            "⚠ 未找到 setupc.exe —— 已搜索 com0com-pro / com0com\n\
                             安装目录、本程序目录与 PATH，均未命中。\n\
                             解决：运行 com0com-pro-setup.exe 勾选\"com0com driver (x64)\"\n\
                             安装，或先安装官方 com0com 3.0。\n\
                             注意：若设备管理器里 com0com 总线设备带黄色感叹号（Code 52），\n\
                             说明测试签名未开启（bcdedit /set testsigning on 后重启）。\n\
                             未装驱动时仍可用 tcp:// 端点体验 bundle。",
                        )
                            .color(egui::Color32::from_rgb(220, 160, 60)),
                    );
                }
                egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                    for p in &self.pairs {
                        ui.label(p);
                    }
                });
            });

        egui::TopBottomPanel::bottom("activity")
            .resizable(true)
            .default_height(220.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("活动日志");
                    ui.label(
                        egui::RichText::new(format!("{} 行", self.activity.len()))
                            .small()
                            .weak(),
                    );
                });
                egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                    for line in &self.activity {
                        ui.label(egui::RichText::new(line).monospace().small());
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(i) = self.selected else {
                ui.centered_and_justified(|ui| {
                    ui.label("左侧选择或新建一个 bundle")
                });
                return;
            };
            let Some(snapshot) = self.cfg.bundles.get(i).cloned() else {
                return;
            };
            ui.heading(format!("编辑：{}", snapshot.name));
            let mut name = snapshot.name.clone();
            let mut kind = snapshot.kind;
            ui.horizontal(|ui| {
                ui.label("名称");
                if ui.text_edit_singleline(&mut name).changed() {
                    if let Some(b) = self.cfg.bundles.get_mut(i) {
                        b.name = name.clone();
                    }
                }
                ui.label("类型");
                egui::ComboBox::from_id_salt("kind")
                    .selected_text(format!("{kind:?}"))
                    .show_ui(ui, |ui| {
                        for k in [
                            BundleKind::Split,
                            BundleKind::Merge,
                            BundleKind::Share,
                            BundleKind::Redirect,
                            BundleKind::Complex,
                        ] {
                            ui.selectable_value(&mut kind, k, format!("{k:?}"));
                        }
                    });
                if kind != snapshot.kind {
                    if let Some(b) = self.cfg.bundles.get_mut(i) {
                        b.kind = kind;
                        b.policy = None;
                    }
                }
            });
            ui.label(
                egui::RichText::new("端点格式：name:端口，TCP 用 name:tcp://host:port")
                    .small()
                    .weak(),
            );
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.label("inlets（数据源侧）");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.inlets_buf)
                            .desired_rows(6)
                            .desired_width(320.0)
                            .font(egui::TextStyle::Monospace),
                    );
                });
                ui.vertical(|ui| {
                    ui.label("outlets（数据消费侧）");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.outlets_buf)
                            .desired_rows(6)
                            .desired_width(320.0)
                            .font(egui::TextStyle::Monospace),
                    );
                });
            });
            ui.horizontal(|ui| {
                ui.label("策略");
                ui.add(
                    egui::TextEdit::singleline(&mut self.policy_buf)
                        .hint_text(format!("留空 = 默认（{}）", kind.default_policy()))
                        .desired_width(220.0),
                );
                ui.label(
                    egui::RichText::new("copy-all / round-robin / one-to-one / first-writer-wins:毫秒")
                        .small()
                        .weak(),
                );
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("应用修改").clicked() {
                    self.apply_buffers();
                    self.status = "edits applied".into();
                }
                if ui.button("恢复").clicked() {
                    self.load_selected_into_buffers();
                }
                if ui.button("校验配置").clicked() {
                    self.apply_buffers();
                    self.status = match self.cfg.validate() {
                        Ok(()) => "config OK".into(),
                        Err(e) => format!("invalid: {e}"),
                    };
                }
            });
        });
    }
}
