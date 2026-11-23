//! 活动日志（JSONL）：端口 open/close、字节计数、信号变化、错误。

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 一条活动日志事件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogEvent {
    /// RFC 3339 UTC 时间戳。
    pub ts: String,
    /// 所属 bundle 名。
    pub bundle: String,
    /// 端点名。
    pub endpoint: String,
    /// 事件类型：open / close / read / write / line-change / error。
    pub event: String,
    /// 字节数（read/write 事件）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<usize>,
    /// 附加说明。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl LogEvent {
    pub fn now(bundle: &str, endpoint: &str, event: &str) -> Self {
        Self {
            ts: rfc3339_now(),
            bundle: bundle.to_string(),
            endpoint: endpoint.to_string(),
            event: event.to_string(),
            bytes: None,
            detail: None,
        }
    }

    pub fn with_bytes(mut self, n: usize) -> Self {
        self.bytes = Some(n);
        self
    }

    pub fn with_detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
}

/// RFC 3339 UTC 时间戳（无外部依赖的最小实现）。
pub fn rfc3339_now() -> String {
    // 仅用于日志可读性；精度到秒。避免引入 chrono（GPL 项目里外部依赖越少越好）。
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// 天数 → 公历日期（Howard Hinnant 算法）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// JSONL 活动日志。可写文件，也可写入任意 sink（测试/内存缓冲）。
pub struct ActivityLog {
    sink: Box<dyn Write + Send>,
    #[allow(dead_code)]
    path: Option<PathBuf>,
}

impl ActivityLog {
    pub fn file(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self {
            sink: Box::new(std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?),
            path: Some(path.to_path_buf()),
        })
    }

    pub fn sink(w: Box<dyn Write + Send>) -> Self {
        Self { sink: w, path: None }
    }

    pub fn record(&mut self, ev: &LogEvent) -> std::io::Result<()> {
        serde_json::to_writer(&mut self.sink, ev)?;
        self.sink.write_all(b"\n")?;
        self.sink.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_shape_and_civil_conversion() {
        let ts = rfc3339_now();
        assert_eq!(ts.len(), 20);
        assert!(ts.ends_with('Z'));
        assert_eq!(ts[4..5].to_string(), "-");
        // 2026-11-23 约为 17650 之后的日期段：仅断言年份四位合理性
        assert!(ts.starts_with("20"), "got {ts}");
    }

    #[test]
    fn epoch_zero_is_1970_01_01() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1)); // 2024-01-01
    }

    #[test]
    fn jsonl_roundtrip_in_memory() {
        use std::sync::{Arc, Mutex};
        #[derive(Clone, Default)]
        struct SharedBuf(Arc<Mutex<Vec<u8>>>);
        impl Write for SharedBuf {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buf = SharedBuf::default();
        let mut log = ActivityLog::sink(Box::new(buf.clone()));
        log.record(&LogEvent::now("b1", "CNCA0", "open").with_detail("app.exe")).unwrap();
        log.record(&LogEvent::now("b1", "CNCA0", "write").with_bytes(128)).unwrap();
        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let ev1: LogEvent = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(ev1.event, "open");
        assert_eq!(ev1.detail.as_deref(), Some("app.exe"));
        let ev2: LogEvent = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(ev2.bytes, Some(128));
    }

    #[test]
    fn event_serializes_expected_fields() {
        let mut ev = LogEvent::now("b", "e", "read");
        ev.ts = "2026-11-23T09:00:00Z".into();
        ev.bytes = Some(42);
        let s = serde_json::to_string(&ev).unwrap();
        assert!(s.contains(r#""bytes":42"#));
        assert!(s.contains(r#""bundle":"b""#));
        assert!(s.contains(r#""event":"read""#));
        assert!(!s.contains("detail"));
    }
}
