//! 端点抽象。
//!
//! bundle 引擎只面向 [`Endpoint`] trait：真实串口（P3 的 SerialEndpoint）、
//! 命名管道、以及单测用的 [`MemoryEndpoint`] 都实现它。写入语义为"尽力写入
//! data 并返回写入字节数"，与 std::io::Write 一致。

use std::io;
use std::sync::{Arc, Mutex};

/// bundle 引擎可见的最小端点接口。
pub trait Endpoint: Send {
    /// 端点名（与 `EndpointSpec::name` 对应，用于日志与路由匹配）。
    fn name(&self) -> &str;
    /// 写入数据，返回实际写入字节数。
    fn write(&mut self, data: &[u8]) -> io::Result<usize>;
}

/// 内存测试端点：写入被记录到 outbox，同时投递到 peer 的 inbox。
///
/// 单测里通过 [`MemoryEndpoint::connect_peer`] 建立链路，用
/// [`MemoryEndpoint::take_written`] 断言路由结果。
#[derive(Clone)]
pub struct MemoryEndpoint {
    name: Arc<str>,
    inner: Arc<Mutex<MemInner>>,
}

#[derive(Default)]
struct MemInner {
    inbox: Vec<u8>,
    written: Vec<u8>,
    /// 模拟写入失败（背压/端口拔出场景）。
    fail_writes: bool,
    peer: Option<Arc<Mutex<MemInner>>>,
}

impl MemoryEndpoint {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: Arc::from(name.into().as_str()),
            inner: Arc::new(Mutex::new(MemInner::default())),
        }
    }

    /// 建立双向链路：self 写入 → peer inbox，peer 写入 → self inbox。
    pub fn connect_peer(&self, peer: &MemoryEndpoint) {
        let mut a = self.inner.lock().unwrap();
        let mut b = peer.inner.lock().unwrap();
        a.peer = Some(Arc::clone(&peer.inner));
        b.peer = Some(Arc::clone(&self.inner));
    }

    /// 取走并清空本端被写入的数据（测试断言用）。
    pub fn take_written(&self) -> Vec<u8> {
        std::mem::take(&mut self.inner.lock().unwrap().written)
    }

    /// 取走 inbox 数据（模拟另一端的应用读走数据）。
    pub fn take_inbox(&self) -> Vec<u8> {
        std::mem::take(&mut self.inner.lock().unwrap().inbox)
    }

    /// 设置写入失败模拟。
    pub fn set_fail_writes(&self, fail: bool) {
        self.inner.lock().unwrap().fail_writes = fail;
    }
}

impl Endpoint for MemoryEndpoint {
    fn name(&self) -> &str {
        &self.name
    }

    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut g = self.inner.lock().unwrap();
        if g.fail_writes {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "fail_writes"));
        }
        g.written.extend_from_slice(data);
        if let Some(peer) = g.peer.clone() {
            peer.lock().unwrap().inbox.extend_from_slice(data);
        }
        Ok(data.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_pair_delivers_both_directions() {
        let mut a = MemoryEndpoint::new("A");
        let mut b = MemoryEndpoint::new("B");
        a.connect_peer(&b);

        a.write(b"hello").unwrap();
        assert_eq!(b.take_inbox(), b"hello");
        assert_eq!(a.take_written(), b"hello");

        b.write(b"world").unwrap();
        assert_eq!(a.take_inbox(), b"world");
    }

    #[test]
    fn fail_writes_yields_broken_pipe() {
        let mut a = MemoryEndpoint::new("A");
        a.set_fail_writes(true);
        let err = a.write(b"x").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn names_are_stable() {
        let a = MemoryEndpoint::new("CNCA0");
        assert_eq!(Endpoint::name(&a), "CNCA0");
    }
}
