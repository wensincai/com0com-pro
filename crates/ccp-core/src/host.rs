//! bundle 运行时宿主：打开端点、起泵线程、驱动引擎循环。
//!
//! 架构（对应 docs/04-架构设计.md §2）：
//!
//! ```text
//!  端点设备 ── 泵线程（阻塞读）──► engine_rx ──► BundleEngine.route()
//!       ▲                                            │
//!       └── 泵线程（命令写）◄── ChannelEndpoint ◄─────┘
//! ```
//!
//! 每个端点一条泵线程：阻塞读设备 → 发给引擎；写命令来自引擎路由输出。
//! 引擎循环单线程串行处理，路由核心无需加锁。

use crate::config::{EndpointKind, EndpointSpec, SessionConfig};
use crate::engine::BundleEngine;
use crate::error::{Error, Result};
use crate::log::{ActivityLog, LogEvent};
use crate::Endpoint;
use serialport::SerialPortType;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 泵线程命令。
pub enum PumpCommand {
    Write(Vec<u8>),
    Close,
}

/// 泵读取超时：兼顾响应性（100 ms 内可处理写命令/停止）与 CPU 占用。
const PUMP_TIMEOUT: Duration = Duration::from_millis(100);

/// 引擎循环收到的一帧：来源端点名 + 数据。
pub type Frame = (String, Vec<u8>);

/// 通道型出口端点：Router 写入 → 转发给对应泵线程。
struct ChannelEndpoint {
    name: Arc<str>,
    tx: Sender<PumpCommand>,
}

impl Endpoint for ChannelEndpoint {
    fn name(&self) -> &str {
        &self.name
    }
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.tx
            .send(PumpCommand::Write(data.to_vec()))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "pump thread gone"))?;
        Ok(data.len())
    }
}

/// 共享停止标志。
#[derive(Clone, Default)]
pub struct StopFlag(Arc<AtomicBool>);

impl StopFlag {
    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_stopped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// 共享活动日志（多泵线程 + 引擎循环写）。
pub type SharedLog = Arc<Mutex<ActivityLog>>;

pub fn shared_log_to(path: &Path) -> Result<SharedLog> {
    Ok(Arc::new(Mutex::new(ActivityLog::file(path)?)))
}

fn log_ev(log: &SharedLog, ev: &LogEvent) {
    let _ = log.lock().map(|mut l| l.record(ev));
}

/// 枚举本机串口（GUI/CLI 列表用）。
pub fn list_serial_ports() -> Result<Vec<String>> {
    let ports = serialport::available_ports()
        .map_err(|e| Error::Config(format!("enumerate serial ports: {e}")))?;
    Ok(ports
        .into_iter()
        .map(|p| match p.port_type {
            SerialPortType::UsbPort(info) => format!("{} (USB {:04x}:{:04x})", p.port_name, info.vid, info.pid),
            SerialPortType::BluetoothPort => format!("{} (Bluetooth)", p.port_name),
            SerialPortType::PciPort => format!("{} (PCI)", p.port_name),
            _ => p.port_name,
        })
        .collect())
}

/// 一个运行中的 bundle 的可见状态。
#[derive(Debug, Clone)]
pub struct BundleStatus {
    pub name: String,
    pub endpoints: Vec<String>,
    pub ok: bool,
    pub detail: String,
}

enum Device {
    Serial(Box<dyn serialport::SerialPort>),
    Tcp(std::net::TcpStream),
    /// 内存设备（测试用）：读 = 取走 inbox，写 = 追加到 outbox。
    #[cfg_attr(not(test), allow(dead_code))]
    Memory(Arc<Mutex<MemDev>>),
}

/// 内存设备状态（测试可克隆 Arc 直接触碰 inbox/outbox）。
#[derive(Default)]
pub struct MemDev {
    pub inbox: Vec<u8>,
    pub outbox: Vec<u8>,
}

impl Device {
    fn set_timeouts(&mut self) {
        match self {
            Device::Serial(p) => {
                let _ = (**p).set_timeout(PUMP_TIMEOUT);
            }
            Device::Tcp(s) => {
                let _ = s.set_read_timeout(Some(PUMP_TIMEOUT));
                let _ = s.set_write_timeout(Some(PUMP_TIMEOUT));
            }
            Device::Memory(_) => {}
        }
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Device::Serial(p) => p.read(buf),
            Device::Tcp(s) => s.read(buf),
            Device::Memory(m) => {
                let mut g = m.lock().unwrap();
                let n = g.inbox.len().min(buf.len());
                buf[..n].copy_from_slice(&g.inbox[..n]);
                g.inbox.drain(..n);
                if n == 0 {
                    // 模拟阻塞读超时
                    Err(io::Error::new(io::ErrorKind::TimedOut, "no data"))
                } else {
                    Ok(n)
                }
            }
        }
    }
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match self {
            Device::Serial(p) => p.write(data),
            Device::Tcp(s) => s.write(data),
            Device::Memory(m) => {
                m.lock().unwrap().outbox.extend_from_slice(data);
                Ok(data.len())
            }
        }
    }
}
// SerialPort trait 方法经由 Box 自动派发（use serialport::SerialPort 已导入）。

/// 按 spec 打开设备。
fn open_device(spec: &EndpointSpec) -> Result<Device> {
    match &spec.kind {
        EndpointKind::Port { port } => {
            let p = serialport::new(port, 115_200)
                .timeout(PUMP_TIMEOUT)
                .open()
                .map_err(|e| Error::Io(io::Error::new(io::ErrorKind::NotFound, e)))?;
            Ok(Device::Serial(p))
        }
        EndpointKind::Tcp { addr } => {
            let s = std::net::TcpStream::connect(addr)?;
            Ok(Device::Tcp(s))
        }
        EndpointKind::Pipe { .. } => Err(Error::Config(
            "pipe endpoints are not wired yet (P4 IPC phase); use port or tcp".into(),
        )),
    }
}

/// 起一条泵线程。
fn spawn_pump(
    endpoint: &EndpointSpec,
    device: Device,
    engine_tx: Sender<Frame>,
) -> (Sender<PumpCommand>, std::thread::JoinHandle<()>) {
    let mut device = device;
    device.set_timeouts();
    let (cmd_tx, cmd_rx) = channel::<PumpCommand>();
    let name = endpoint.name.clone();
    let join = std::thread::Builder::new()
        .name(format!("pump-{}", name))
        .spawn(move || {
            let mut dev = device;
            loop {
                // 写命令优先（非阻塞）
                match cmd_rx.try_recv() {
                    Ok(PumpCommand::Write(data)) => {
                        if let Err(e) = dev.write(&data) {
                            let _ = engine_tx.send((
                                format!("!{name}"),
                                format!("write error: {e}").into_bytes(),
                            ));
                        }
                        continue;
                    }
                    Ok(PumpCommand::Close) | Err(TryRecvError::Disconnected) => break,
                    Err(TryRecvError::Empty) => {}
                }
                // 阻塞读（带超时）
                let mut buf = [0u8; 4096];
                match dev.read(&mut buf) {
                    Ok(0) => break, // 对端关闭
                    Ok(n) => {
                        if engine_tx.send((name.clone(), buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(e)
                        if e.kind() == io::ErrorKind::TimedOut
                            || e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(_) => break, // 设备拔出/断开
                }
            }
        })
        .expect("spawn pump thread");
    (cmd_tx, join)
}

/// 运行一个 bundle 直到 stop；返回状态快照。
///
/// `preopen`：测试注入的设备表（按端点名），命中时跳过真实设备打开。
fn run_bundle(
    spec: crate::config::BundleSpec,
    log: &SharedLog,
    stop: &StopFlag,
    preopen: Option<&std::collections::HashMap<String, Device>>,
) -> Result<BundleStatus> {
    // 1. 打开全部端点（bundle 的每个端口都既收又发）
    let mut needed: Vec<&EndpointSpec> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for e in spec.inlets.iter().chain(spec.outlets.iter()) {
        if !seen.contains(&e.name) {
            seen.push(e.name.clone());
            needed.push(e);
        }
    }
    let mut devices = Vec::new();
    let mut opened: Vec<String> = Vec::new();
    for e in &needed {
        let dev = match preopen.and_then(|m| m.get(&e.name)) {
            Some(d) => {
                // clone 内存设备句柄（Arc 共享）
                match d {
                    Device::Memory(m) => Device::Memory(Arc::clone(m)),
                    _ => return Err(Error::Config("preopen supports memory devices".into())),
                }
            }
            None => match open_device(e) {
                Ok(d) => d,
                Err(err) => {
                    return Err(Error::Router(format!(
                        "endpoint `{}` open failed: {err}",
                        e.name
                    )));
                }
            },
        };
        devices.push(dev);
        opened.push(e.name.clone());
    }

    // 2. 引擎 + 泵
    let (engine_tx, engine_rx) = channel::<Frame>();
    let mut pump_txs = Vec::new();
    let mut pump_joins = Vec::new();
    for (dev, e) in devices.into_iter().zip(needed.iter()) {
        let (tx, join) = spawn_pump(e, dev, engine_tx.clone());
        pump_txs.push(tx);
        pump_joins.push(join);
    }

    let mut engine = BundleEngine::build(
        spec.clone(),
        needed
            .iter()
            .cloned()
            .zip(pump_txs.iter().cloned())
            .map(|(e, tx)| Box::new(ChannelEndpoint {
                name: Arc::from(e.name.as_str()),
                tx,
            }) as Box<dyn Endpoint>)
            .collect(),
    )?;
    let log2 = Arc::clone(log);
    engine.set_event_sink(Box::new(move |ev| log_ev(&log2, ev)));
    log_ev(log, &LogEvent::now(&spec.name, "-", "bundle-open"));

    // 3. 引擎循环（本线程内，直到 stop 或泵死光）
    let mut ok = true;
    let mut detail = String::new();
    while !stop.is_stopped() {
        match engine_rx.recv_timeout(PUMP_TIMEOUT) {
            Ok((inlet, data)) => {
                if let Some(peer) = inlet.strip_prefix('!') {
                    // 泵线程错误上报
                    log_ev(
                        log,
                        &LogEvent::now(&spec.name, peer, "error")
                            .with_detail(String::from_utf8_lossy(&data).to_string()),
                    );
                    ok = false;
                    detail = String::from_utf8_lossy(&data).to_string();
                    break;
                }
                if let Err(e) = engine.route(&inlet, &data) {
                    detail = e.to_string();
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                detail = "all pumps gone".into();
                ok = false;
                break;
            }
        }
    }

    // 4. 收尾：通知泵关闭、等线程
    for tx in &pump_txs {
        let _ = tx.send(PumpCommand::Close);
    }
    for j in pump_joins {
        let _ = j.join();
    }
    for e in &opened {
        log_ev(log, &LogEvent::now(&spec.name, e, "close"));
    }
    log_ev(log, &LogEvent::now(&spec.name, "-", "bundle-close"));
    if !ok {
        return Err(Error::Router(detail));
    }
    Ok(BundleStatus {
        name: spec.name,
        endpoints: opened,
        ok,
        detail,
    })
}

/// 运行整个会话：逐 bundle 在独立线程中 run_bundle。
/// 阻塞直到 stop 被置位。返回各 bundle 的最终状态。
pub fn run_session(config: &SessionConfig, log: &SharedLog, stop: &StopFlag) -> Vec<BundleStatus> {
    let mut joins = Vec::new();
    for spec in &config.bundles {
        let spec = spec.clone();
        let log = Arc::clone(log);
        let stop = stop.clone();
        joins.push(std::thread::Builder::new()
            .name(format!("bundle-{}", spec.name))
            .spawn(move || run_bundle(spec, &log, &stop, None))
            .expect("spawn bundle thread"));
    }
    let mut out = Vec::new();
    for j in joins {
        match j.join() {
            Ok(Ok(status)) => out.push(status),
            Ok(Err(e)) => out.push(BundleStatus {
                name: "?".into(),
                endpoints: vec![],
                ok: false,
                detail: e.to_string(),
            }),
            Err(_) => out.push(BundleStatus {
                name: "?".into(),
                endpoints: vec![],
                ok: false,
                detail: "bundle thread panicked".into(),
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BundleKind, BundleSpec, EndpointKind, EndpointSpec};
    use std::collections::HashMap;

    fn port_spec(name: &str, port: &str) -> EndpointSpec {
        EndpointSpec {
            name: name.into(),
            kind: EndpointKind::Port { port: port.into() },
        }
    }

    fn mem_device() -> (Device, Arc<Mutex<MemDev>>) {
        let m = Arc::new(Mutex::new(MemDev::default()));
        (Device::Memory(Arc::clone(&m)), m)
    }

    /// 等待直到条件成立或超时（泵线程异步，需轮询）。
    fn wait_for(timeout_ms: u64, mut f: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
        while std::time::Instant::now() < deadline {
            if f() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn redirect_pumps_route_in_both_directions() {
        // A ↔ B：应用写 A 设备 inbox → 应出现在 B 设备 outbox；反向同理。
        let (dev_a, a) = mem_device();
        let (dev_b, b) = mem_device();
        let spec = BundleSpec {
            name: "link".into(),
            kind: BundleKind::Redirect,
            inlets: vec![port_spec("A", "COM5")],
            outlets: vec![port_spec("B", "COM6")],
            policy: Some("one-to-one".into()),
            acl: vec![],
        };
        let mut preopen = HashMap::new();
        preopen.insert("A".into(), dev_a);
        preopen.insert("B".into(), dev_b);

        let log = Arc::new(Mutex::new(ActivityLog::sink(Box::new(Vec::<u8>::new()))));
        let stop = StopFlag::default();
        let runner = {
            let log = Arc::clone(&log);
            let stop = stop.clone();
            let spec = spec.clone();
            std::thread::spawn(move || run_bundle(spec, &log, &stop, Some(&preopen)))
        };
        std::thread::sleep(Duration::from_millis(200));

        // 应用侧：向 A 写数据（模拟对端应用输出）
        a.lock().unwrap().inbox.extend_from_slice(b"ping");
        wait_for(2000, || {
            String::from_utf8_lossy(&b.lock().unwrap().outbox).contains("ping")
        });
        assert_eq!(b.lock().unwrap().outbox, b"ping");

        // 反向
        b.lock().unwrap().inbox.extend_from_slice(b"pong");
        wait_for(2000, || {
            String::from_utf8_lossy(&a.lock().unwrap().outbox).contains("pong")
        });
        assert_eq!(a.lock().unwrap().outbox, b"pong");

        stop.stop();
        let status = runner.join().unwrap().unwrap();
        assert!(status.ok);
    }

    #[test]
    fn split_pump_copies_to_all_outlets() {
        let (dev_in, src) = mem_device();
        let (dev_o1, o1) = mem_device();
        let (dev_o2, o2) = mem_device();
        let spec = BundleSpec {
            name: "split".into(),
            kind: BundleKind::Split,
            inlets: vec![port_spec("dev", "COM3")],
            outlets: vec![port_spec("o1", "CNCA0"), port_spec("o2", "CNCA1")],
            policy: None,
            acl: vec![],
        };
        let mut preopen = HashMap::new();
        preopen.insert("dev".into(), dev_in);
        preopen.insert("o1".into(), dev_o1);
        preopen.insert("o2".into(), dev_o2);

        let log = Arc::new(Mutex::new(ActivityLog::sink(Box::new(Vec::<u8>::new()))));
        let stop = StopFlag::default();
        let runner = {
            let log = Arc::clone(&log);
            let stop = stop.clone();
            let spec = spec.clone();
            std::thread::spawn(move || run_bundle(spec, &log, &stop, Some(&preopen)))
        };
        std::thread::sleep(Duration::from_millis(200));

        src.lock().unwrap().inbox.extend_from_slice(b"NMEA");
        wait_for(2000, || {
            o1.lock().unwrap().outbox == b"NMEA".to_vec()
                && o2.lock().unwrap().outbox == b"NMEA".to_vec()
        });
        assert_eq!(o1.lock().unwrap().outbox, b"NMEA");
        assert_eq!(o2.lock().unwrap().outbox, b"NMEA");
        stop.stop();
        assert!(runner.join().unwrap().unwrap().ok);
    }

    #[test]
    fn share_policy_first_writer_wins_over_pumps() {
        let (dev_a, a) = mem_device();
        let (dev_b, b_in) = mem_device();
        let (dev_o, o) = mem_device();
        let spec = BundleSpec {
            name: "share".into(),
            kind: BundleKind::Share,
            inlets: vec![port_spec("a", "COM7"), port_spec("b", "COM8")],
            outlets: vec![port_spec("dev", "COM9")],
            policy: Some("first-writer-wins:60000".into()),
            acl: vec![],
        };
        let mut preopen = HashMap::new();
        preopen.insert("a".into(), dev_a);
        preopen.insert("b".into(), dev_b);
        preopen.insert("dev".into(), dev_o);

        let log = Arc::new(Mutex::new(ActivityLog::sink(Box::new(Vec::<u8>::new()))));
        let stop = StopFlag::default();
        let runner = {
            let log = Arc::clone(&log);
            let stop = stop.clone();
            let spec = spec.clone();
            std::thread::spawn(move || run_bundle(spec, &log, &stop, Some(&preopen)))
        };
        std::thread::sleep(Duration::from_millis(200));

        a.lock().unwrap().inbox.extend_from_slice(b"from-a");
        wait_for(2000, || !o.lock().unwrap().outbox.is_empty());
        assert_eq!(o.lock().unwrap().outbox, b"from-a");

        // b 写入被丢弃（a 持有）
        b_in.lock().unwrap().inbox.extend_from_slice(b"from-b");
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(o.lock().unwrap().outbox, b"from-a");

        stop.stop();
        assert!(runner.join().unwrap().unwrap().ok);
    }

    #[test]
    fn run_session_reports_open_failure() {
        let spec = BundleSpec {
            name: "bad".into(),
            kind: BundleKind::Split,
            inlets: vec![port_spec("i", "COM-NOTEXIST")],
            outlets: vec![port_spec("o", "COM-ALSO")],
            policy: None,
            acl: vec![],
        };
        let cfg = SessionConfig { bundles: vec![spec] };
        let log = Arc::new(Mutex::new(ActivityLog::sink(Box::new(Vec::<u8>::new()))));
        let stop = StopFlag::default();
        let statuses = run_session(&cfg, &log, &stop);
        assert!(!statuses[0].ok);
        assert!(statuses[0].detail.contains("open failed"));
    }

    #[test]
    fn serial_port_enumeration_does_not_panic() {
        // 无串口的 CI 环境返回空列表也不应出错
        let _ = list_serial_ports();
    }

    /// 手动 E2E（需本机 TCP 可绑定）：`cargo test -p ccp-core -- --ignored`
    /// 两个 echo server + redirect bundle，写 A 应回到 A（经 B echo）。
    #[test]
    #[ignore = "binds local TCP listeners; run manually"]
    fn redirect_loopback_over_two_tcp_echo_servers() {
        use std::io::{Read, Write};
        fn spawn_echo(addr: &'static str) {
            let listener = std::net::TcpListener::bind(addr).unwrap();
            std::thread::spawn(move || {
                if let Ok((mut s, _)) = listener.accept() {
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = s.read(&mut buf) {
                        if n == 0 || s.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                }
            });
        }
        spawn_echo("127.0.0.1:19001");
        spawn_echo("127.0.0.1:19002");
        let tcp = |name: &str, addr: &str| EndpointSpec {
            name: name.into(),
            kind: EndpointKind::Tcp { addr: addr.into() },
        };
        let spec = BundleSpec {
            name: "tcp-loop".into(),
            kind: BundleKind::Redirect,
            inlets: vec![tcp("A", "127.0.0.1:19001")],
            outlets: vec![tcp("B", "127.0.0.1:19002")],
            policy: Some("one-to-one".into()),
            acl: vec![],
        };
        let log = Arc::new(Mutex::new(ActivityLog::sink(Box::new(Vec::<u8>::new()))));
        let stop = StopFlag::default();
        let runner = {
            let log = Arc::clone(&log);
            let stop = stop.clone();
            let spec = spec.clone();
            std::thread::spawn(move || run_bundle(spec, &log, &stop, None))
        };
        std::thread::sleep(Duration::from_millis(300));

        // 应用侧直连 A 的 echo server，写数据 → bundle 把它送进 B → echo 回 → 回到 A
        let mut app = std::net::TcpStream::connect("127.0.0.1:19001").unwrap();
        app.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        app.write_all(b"hello-loop").unwrap();
        let mut buf = [0u8; 64];
        let n = app.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello-loop");
        stop.stop();
        assert!(runner.join().unwrap().unwrap().ok);
    }
}
