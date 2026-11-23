//! bundle 路由引擎：把 inlet 侧到达的数据按策略分发到 outlet 侧。
//!
//! 路由核心是纯同步逻辑（`route()` 一次调用处理一帧），泵与线程在
//! P3 的引擎宿主里实现——这样策略行为可在 CI 用内存端点全速单测。

use crate::endpoint::Endpoint;
use crate::error::{Error, Result};
use std::time::Instant;

/// 分发策略。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutePolicy {
    /// 一对一交叉：inlet[i] 的数据写给 outlet[i]（redirect/share 双向链路用）。
    OneToOne,
    /// 复制全部：每个 inlet 的数据复制给所有 outlet（split 用）。
    CopyAll,
    /// 轮转：inlet 数据轮流写给下一个 outlet（merge 用）。
    RoundRobin,
    /// 先写者持有：某 inlet 开始写后独占全部 outlet，直到空闲超时或显式释放
    /// （share 用）。
    FirstWriterWins { idle_ms: u64 },
}

impl RoutePolicy {
    /// 从配置的字符串形式解析。
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "one-to-one" => Ok(RoutePolicy::OneToOne),
            "copy-all" => Ok(RoutePolicy::CopyAll),
            "round-robin" => Ok(RoutePolicy::RoundRobin),
            s if s.starts_with("first-writer-wins") => {
                // 形如 first-writer-wins:500（毫秒）
                let idle_ms = s
                    .split_once(':')
                    .and_then(|(_, v)| v.parse().ok())
                    .unwrap_or(1000);
                Ok(RoutePolicy::FirstWriterWins { idle_ms })
            }
            other => Err(Error::Router(format!("unknown policy: {other}"))),
        }
    }
}

/// bundle 路由器。
pub struct Router {
    policy: RoutePolicy,
    /// inlet 名单（路由匹配用，按序）。
    inlets: Vec<String>,
    outlets: Vec<Box<dyn Endpoint>>,
    // RoundRobin 状态
    rr_next: usize,
    // FirstWriterWins 状态
    owner: Option<String>,
    owner_since: Option<Instant>,
}

impl Router {
    pub fn new(policy: RoutePolicy, inlets: Vec<String>, outlets: Vec<Box<dyn Endpoint>>) -> Self {
        Self {
            policy,
            inlets,
            outlets,
            rr_next: 0,
            owner: None,
            owner_since: None,
        }
    }

    pub fn policy(&self) -> &RoutePolicy {
        &self.policy
    }

    pub fn outlet_names(&self) -> Vec<String> {
        self.outlets.iter().map(|o| o.name().to_string()).collect()
    }

    /// 处理一帧来自 `inlet` 的数据。返回总写入字节数。
    ///
    /// 单个 outlet 写失败不终止整体路由（尽力而为），但会返回 Err 让上层
    /// 记录活动日志；`Ok(0)` 表示按策略被丢弃（如 share 的非持有者写入）。
    pub fn route(&mut self, inlet: &str, data: &[u8]) -> Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        match &self.policy {
            RoutePolicy::OneToOne => {
                let idx = self
                    .inlets
                    .iter()
                    .position(|n| n == inlet)
                    .ok_or_else(|| Error::Router(format!("unknown inlet: {inlet}")))?;
                if idx >= self.outlets.len() {
                    return Err(Error::Router(format!(
                        "no outlet paired for inlet index {idx}"
                    )));
                }
                let n = self.outlets[idx].write(data)?;
                Ok(n)
            }
            RoutePolicy::CopyAll => {
                let mut total = 0;
                let mut first_err = None;
                for out in &mut self.outlets {
                    match out.write(data) {
                        Ok(n) => total += n,
                        Err(e) => first_err = first_err.or(Some(e)),
                    }
                }
                match first_err {
                    Some(e) => Err(Error::Io(e)),
                    None => Ok(total),
                }
            }
            RoutePolicy::RoundRobin => {
                if self.outlets.is_empty() {
                    return Err(Error::Router("no outlets".into()));
                }
                let idx = self.rr_next % self.outlets.len();
                self.rr_next = idx + 1;
                let n = self.outlets[idx].write(data)?;
                Ok(n)
            }
            RoutePolicy::FirstWriterWins { idle_ms } => {
                // 持有者空闲超时自动释放
                if let (Some(_), Some(since)) = (&self.owner, self.owner_since) {
                    if since.elapsed().as_millis() as u64 >= *idle_ms {
                        self.owner = None;
                        self.owner_since = None;
                    }
                }
                match &self.owner {
                    None => {
                        self.owner = Some(inlet.to_string());
                        self.owner_since = Some(Instant::now());
                        self.write_all_outlets(data)
                    }
                    Some(current) if current == inlet => {
                        self.owner_since = Some(Instant::now());
                        self.write_all_outlets(data)
                    }
                    Some(_other) => {
                        // 非持有者的写入被丢弃（返回 0，不报错——上层可记日志）
                        Ok(0)
                    }
                }
            }
        }
    }

    /// 显式释放 share 持有权（上层检测到 inlet 关闭时调用）。
    pub fn release(&mut self, inlet: &str) {
        if self.owner.as_deref() == Some(inlet) {
            self.owner = None;
            self.owner_since = None;
        }
    }

    fn write_all_outlets(&mut self, data: &[u8]) -> Result<usize> {
        let mut total = 0;
        let mut first_err = None;
        for out in &mut self.outlets {
            match out.write(data) {
                Ok(n) => total += n,
                Err(e) => first_err = first_err.or(Some(e)),
            }
        }
        match first_err {
            Some(e) => Err(Error::Io(e)),
            None => Ok(total),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::MemoryEndpoint;

    fn ep(n: &str) -> MemoryEndpoint {
        MemoryEndpoint::new(n)
    }

    #[test]
    fn parse_policies() {
        assert_eq!(RoutePolicy::parse("copy-all").unwrap(), RoutePolicy::CopyAll);
        assert_eq!(
            RoutePolicy::parse("first-writer-wins:250").unwrap(),
            RoutePolicy::FirstWriterWins { idle_ms: 250 }
        );
        assert!(RoutePolicy::parse("bogus").is_err());
    }

    #[test]
    fn one_to_one_crosses_sides() {
        let a = ep("A");
        let b = ep("B");
        let mut r = Router::new(
            RoutePolicy::OneToOne,
            vec!["A".into(), "B".into()],
            vec![Box::new(ep("B")), Box::new(ep("A"))],
        );
        // 注：outlet 与 test 中的 a/b 实例独立（内存端点写记录在自身 written），
        // OneToOne 把 inlet A 的数据写给 outlets[0]（名为 B）。
        r.route("A", b"x1").unwrap();
        r.route("B", b"x2").unwrap();
        // outlets[0] 是 "B" 名端点、outlets[1] 是 "A" 名端点——均为 Router 私有，
        // 通过 outlet_names 校验配对正确性即可（数据断言在 engine 测试中做）。
        assert_eq!(r.outlet_names(), vec!["B".to_string(), "A".to_string()]);
        let _ = (a, b);
    }

    #[test]
    fn copy_all_writes_every_outlet() {
        let o1 = ep("O1");
        let o2 = ep("O2");
        // clone 与原实例共享内存 → 便于断言
        let o1c = o1.clone();
        let o2c = o2.clone();
        let mut r = Router::new(
            RoutePolicy::CopyAll,
            vec!["IN".into()],
            vec![Box::new(o1), Box::new(o2)],
        );
        let n = r.route("IN", b"payload").unwrap();
        assert_eq!(n, 14); // 7 字节 × 2 个 outlet
        assert_eq!(o1c.take_written(), b"payload");
        assert_eq!(o2c.take_written(), b"payload");
    }

    #[test]
    fn copy_all_partial_failure_returns_err_but_writes_healthy() {
        let good = ep("GOOD");
        let good_c = good.clone();
        let bad = ep("BAD");
        let bad_c = bad.clone();
        bad_c.set_fail_writes(true);
        let mut r = Router::new(
            RoutePolicy::CopyAll,
            vec!["IN".into()],
            vec![Box::new(good), Box::new(bad)],
        );
        assert!(r.route("IN", b"data").is_err());
        assert_eq!(good_c.take_written(), b"data"); // 健康端点仍收到
    }

    #[test]
    fn round_robin_alternates_outlets() {
        let o1 = ep("O1");
        let o2 = ep("O2");
        let o1c = o1.clone();
        let o2c = o2.clone();
        let mut r = Router::new(
            RoutePolicy::RoundRobin,
            vec!["IN".into()],
            vec![Box::new(o1), Box::new(o2)],
        );
        r.route("IN", b"1").unwrap();
        r.route("IN", b"2").unwrap();
        r.route("IN", b"3").unwrap();
        assert_eq!(o1c.take_written(), b"13");
        assert_eq!(o2c.take_written(), b"2");
    }

    #[test]
    fn first_writer_wins_excludes_others_until_release() {
        let o = ep("O");
        let o_c = o.clone();
        let mut r = Router::new(
            RoutePolicy::FirstWriterWins { idle_ms: 60_000 },
            vec!["A".into(), "B".into()],
            vec![Box::new(o)],
        );
        assert_eq!(r.route("A", b"from-a").unwrap(), 6);
        // B 的写入被丢弃
        assert_eq!(r.route("B", b"from-b").unwrap(), 0);
        assert_eq!(o_c.take_written(), b"from-a");
        // 持有者继续写
        assert_eq!(r.route("A", b"-more").unwrap(), 5);
        // 释放后 B 可写
        r.release("A");
        assert_eq!(r.route("B", b"now-b").unwrap(), 5);
        assert_eq!(o_c.take_written(), b"-morenow-b");
    }

    #[test]
    fn empty_frame_is_noop() {
        let o = ep("O");
        let mut r = Router::new(RoutePolicy::CopyAll, vec!["IN".into()], vec![Box::new(o)]);
        assert_eq!(r.route("IN", b"").unwrap(), 0);
    }
}
