//! bundle 引擎：把 [`BundleSpec`] 组装为可路由的运行时实例。
//!
//! P2 范围：拓扑组装 + 数据路由 + 活动记录回调。真实端点（串口/管道）
//! 由 P3 引擎宿主通过 `Endpoint` trait 注入；单测用 `MemoryEndpoint`。

use crate::config::{BundleKind, BundleSpec};
use crate::endpoint::Endpoint;
use crate::error::{Error, Result};
use crate::log::LogEvent;
use crate::router::{RoutePolicy, Router};

/// 活动事件回调类型。
pub type EventSink = Box<dyn Fn(&LogEvent) + Send>;

/// 运行中的 bundle。
pub struct BundleEngine {
    spec: BundleSpec,
    router: Router,
    /// 活动记录回调（注入式，避免本 crate 依赖具体 I/O）。
    on_event: Option<EventSink>,
}

impl std::fmt::Debug for BundleEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BundleEngine")
            .field("spec", &self.spec)
            .field("policy", self.router.policy())
            .finish_non_exhaustive()
    }
}

impl BundleEngine {
    /// 组装引擎。
    ///
    /// `endpoints` 需覆盖：Split/Merge/Share → 全部 `spec.outlets` 名字；
    /// Redirect → inlet 与 outlet 两侧共 2 个端点（A↔B 双向）。
    pub fn build(spec: BundleSpec, endpoints: Vec<Box<dyn Endpoint>>) -> Result<Self> {
        spec.validate()?;
        let policy = RoutePolicy::parse(spec.effective_policy())?;
        let (router_inlets, want): (Vec<String>, Vec<String>) = match spec.kind {
            BundleKind::Redirect => {
                let a = spec.inlets[0].name.clone();
                let b = spec.outlets[0].name.clone();
                // router: inlet A → outlet B，inlet B → outlet A
                (vec![a.clone(), b.clone()], vec![b, a])
            }
            _ => (
                spec.inlets.iter().map(|i| i.name.clone()).collect(),
                spec.outlets.iter().map(|o| o.name.clone()).collect(),
            ),
        };
        let names: Vec<String> = endpoints.iter().map(|o| o.name().to_string()).collect();
        for w in &want {
            if !names.contains(w) {
                return Err(Error::Router(format!(
                    "endpoint `{w}` missing from supplied endpoints (have {names:?})"
                )));
            }
        }
        // OneToOne 要求 outlet 与 router_inlets 按序配对
        let ordered = if matches!(policy, RoutePolicy::OneToOne) {
            let mut ordered: Vec<Box<dyn Endpoint>> = Vec::with_capacity(want.len());
            let mut supplied = endpoints;
            let mut supplied_names = names;
            for w in &want {
                let pos = supplied_names.iter().position(|n| n == w).ok_or_else(|| {
                    Error::Router(format!("endpoint `{w}` missing from supplied endpoints"))
                })?;
                supplied_names.swap_remove(pos);
                ordered.push(supplied.swap_remove(pos));
            }
            ordered
        } else {
            endpoints
        };
        Ok(Self {
            spec,
            router: Router::new(policy, router_inlets, ordered),
            on_event: None,
        })
    }

    pub fn spec(&self) -> &BundleSpec {
        &self.spec
    }

    pub fn policy(&self) -> &RoutePolicy {
        self.router.policy()
    }

    /// 注入活动记录回调。
    pub fn set_event_sink(&mut self, sink: EventSink) {
        self.on_event = Some(sink);
    }

    /// 路由一帧 inlet 数据，返回总写入字节数；同时记录活动。
    pub fn route(&mut self, inlet: &str, data: &[u8]) -> Result<usize> {
        let res = self.router.route(inlet, data);
        if let Some(sink) = &self.on_event {
            let ev = match &res {
                Err(e) => LogEvent::now(&self.spec.name, inlet, "error")
                    .with_detail(e.to_string()),
                Ok(n) => {
                    if *n > 0 {
                        LogEvent::now(&self.spec.name, inlet, "write").with_bytes(*n)
                    } else {
                        LogEvent::now(&self.spec.name, inlet, "drop")
                            .with_bytes(data.len())
                            .with_detail("policy excluded this inlet")
                    }
                }
            };
            sink(&ev);
        }
        res
    }

    /// inlet 关闭时释放 share 持有权。
    pub fn inlet_closed(&mut self, inlet: &str) {
        self.router.release(inlet);
        if let Some(sink) = &self.on_event {
            sink(&LogEvent::now(&self.spec.name, inlet, "close"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BundleKind, EndpointKind, EndpointSpec};
    use crate::endpoint::MemoryEndpoint;

    fn espec(name: &str, port: &str) -> EndpointSpec {
        EndpointSpec {
            name: name.into(),
            kind: EndpointKind::Port { port: port.into() },
        }
    }

    fn mem(name: &str) -> MemoryEndpoint {
        MemoryEndpoint::new(name)
    }

    #[test]
    fn split_engine_fans_out_and_logs() {
        use std::sync::{Arc, Mutex};
        let events: Arc<Mutex<Vec<LogEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_c = Arc::clone(&events);

        let o1 = mem("app1");
        let o2 = mem("app2");
        let o1c = o1.clone();
        let o2c = o2.clone();

        let spec = BundleSpec {
            name: "gps".into(),
            kind: BundleKind::Split,
            inlets: vec![espec("dev", "COM3")],
            outlets: vec![espec("app1", "CNCA0"), espec("app2", "CNCA1")],
            policy: None,
            acl: vec![],
        };
        let mut eng = BundleEngine::build(spec, vec![Box::new(o1), Box::new(o2)]).unwrap();
        assert_eq!(*eng.policy(), RoutePolicy::CopyAll);
        eng.set_event_sink(Box::new(move |ev| events_c.lock().unwrap().push(ev.clone())));

        assert_eq!(eng.route("dev", b"NMEA$GPGGA").unwrap(), 20); // 2 个 outlet × 10
        assert_eq!(o1c.take_written(), b"NMEA$GPGGA");
        assert_eq!(o2c.take_written(), b"NMEA$GPGGA");

        let evs = events.lock().unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].event, "write");
        assert_eq!(evs[0].bytes, Some(20));
    }

    #[test]
    fn redirect_engine_bidirectional() {
        let a = mem("A"); // outlet 供给：B 侧写出的数据出现在 A
        let b = mem("B");
        let a_c = a.clone();
        let b_c = b.clone();
        let spec = BundleSpec {
            name: "link".into(),
            kind: BundleKind::Redirect,
            inlets: vec![espec("A", "COM5")],
            outlets: vec![espec("B", "-")],
            policy: Some("one-to-one".into()),
            acl: vec![],
        };
        let mut eng =
            BundleEngine::build(spec, vec![Box::new(a), Box::new(b)]).unwrap();
        eng.route("A", b"ping").unwrap();
        eng.route("B", b"pong").unwrap();
        assert_eq!(b_c.take_written(), b"ping"); // A 侧数据 → B
        assert_eq!(a_c.take_written(), b"pong"); // B 侧数据 → A
    }

    #[test]
    fn build_rejects_missing_outlet() {
        let spec = BundleSpec {
            name: "x".into(),
            kind: BundleKind::Split,
            inlets: vec![espec("i", "COM1")],
            outlets: vec![espec("missing", "COM2")],
            policy: None,
            acl: vec![],
        };
        let err = BundleEngine::build(spec, vec![Box::new(mem("other"))]).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn inlet_closed_emits_close_event() {
        use std::sync::{Arc, Mutex};
        let events: Arc<Mutex<Vec<LogEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_c = Arc::clone(&events);
        let spec = BundleSpec {
            name: "s".into(),
            kind: BundleKind::Share,
            inlets: vec![espec("a", "COM1"), espec("b", "COM2")],
            outlets: vec![espec("dev", "COM3")],
            policy: None,
            acl: vec![],
        };
        let mut eng = BundleEngine::build(spec, vec![Box::new(mem("dev"))]).unwrap();
        eng.set_event_sink(Box::new(move |ev| events_c.lock().unwrap().push(ev.clone())));
        assert_eq!(eng.route("a", b"hi").unwrap(), 2);
        eng.inlet_closed("a");
        let evs = events.lock().unwrap();
        assert_eq!(evs.last().unwrap().event, "close");
    }
}
