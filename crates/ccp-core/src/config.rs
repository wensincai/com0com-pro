//! 配置模型与持久化（bundles.json）。
//!
//! 会话配置描述所有 bundle 的拓扑；引擎宿主（P3/P4 的服务）按它重建链路。

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 端点类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum EndpointKind {
    /// com0com 虚拟口或真实 COM 口（如 "CNCA0"、"COM3"）。
    Port { port: String },
    /// 命名管道（P4/IPC 阶段实现）。
    Pipe { path: String },
    /// TCP 端点（host:port；用于 E2E 测试与网络串口桥）。
    Tcp { addr: String },
}

/// 端点规格。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSpec {
    pub name: String,
    #[serde(flatten)]
    pub kind: EndpointKind,
}

/// bundle 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BundleKind {
    /// 1 → N 复制。
    Split,
    /// N → 1 汇合。
    Merge,
    /// N 应用共享 1 端口。
    Share,
    /// A ↔ B 双向转发。
    Redirect,
    /// 自定义组合（P4+）。
    Complex,
}

impl BundleKind {
    /// kind → 默认路由策略。
    pub fn default_policy(self) -> &'static str {
        match self {
            BundleKind::Split => "copy-all",
            BundleKind::Merge => "round-robin",
            BundleKind::Share => "first-writer-wins:1000",
            BundleKind::Redirect => "one-to-one",
            BundleKind::Complex => "copy-all",
        }
    }
}

/// 路由策略配置（字符串形式，见 router::RoutePolicy::parse）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutePolicySpec(pub String);

impl RoutePolicySpec {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 审计式 ACL 规则（P2 记录、P3+ 引擎侧强制）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AclRule {
    /// 进程可执行名（如 "app.exe"），"*" 匹配全部。
    pub process: String,
    /// read / write / both。
    pub access: String,
}

/// 一个 bundle 的完整规格。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleSpec {
    pub name: String,
    pub kind: BundleKind,
    pub inlets: Vec<EndpointSpec>,
    pub outlets: Vec<EndpointSpec>,
    /// 缺省时取 kind 的默认策略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acl: Vec<AclRule>,
}

impl BundleSpec {
    pub fn effective_policy(&self) -> &str {
        match &self.policy {
            Some(p) => p,
            None => self.kind.default_policy(),
        }
    }

    /// 结构校验：名字非空、端点足够、名字唯一。
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(Error::Config("bundle name must not be empty".into()));
        }
        let need = match self.kind {
            BundleKind::Split | BundleKind::Merge => (1, 1),
            BundleKind::Share => (1, 1),
            // Redirect：A ↔ B，配置为 1 个 inlet + 1 个 outlet，引擎自动补反向
            BundleKind::Redirect => (1, 1),
            BundleKind::Complex => (1, 1),
        };
        if self.inlets.len() < need.0 {
            return Err(Error::Config(format!(
                "bundle `{}` needs ≥{} inlet(s), got {}",
                self.name,
                need.0,
                self.inlets.len()
            )));
        }
        if self.outlets.len() < need.1 {
            return Err(Error::Config(format!(
                "bundle `{}` needs ≥{} outlet(s), got {}",
                self.name,
                need.1,
                self.outlets.len()
            )));
        }
        // 同名允许跨侧出现（Redirect 的 A/B 既是 inlet 又是 outlet），
        // 但每一侧内部必须唯一
        let mut inlet_names: Vec<&str> = Vec::new();
        for e in &self.inlets {
            if e.name.is_empty() {
                return Err(Error::Config(format!(
                    "bundle `{}` has an endpoint without a name",
                    self.name
                )));
            }
            if !inlet_names.contains(&e.name.as_str()) {
                inlet_names.push(&e.name);
            } else {
                return Err(Error::Config(format!(
                    "bundle `{}` has duplicate inlet name `{}`",
                    self.name, e.name
                )));
            }
        }
        let mut outlet_names: Vec<&str> = Vec::new();
        for e in &self.outlets {
            if e.name.is_empty() {
                return Err(Error::Config(format!(
                    "bundle `{}` has an endpoint without a name",
                    self.name
                )));
            }
            if !outlet_names.contains(&e.name.as_str()) {
                outlet_names.push(&e.name);
            } else {
                return Err(Error::Config(format!(
                    "bundle `{}` has duplicate outlet name `{}`",
                    self.name, e.name
                )));
            }
        }
        if self.kind == BundleKind::Redirect {
            if self.inlets.len() != 1 || self.outlets.len() != 1 {
                return Err(Error::Config(format!(
                    "bundle `{}` (Redirect) must have exactly 1 inlet and 1 outlet",
                    self.name
                )));
            }
            if self.inlets[0].name == self.outlets[0].name {
                return Err(Error::Config(format!(
                    "bundle `{}` (Redirect) inlet and outlet must differ",
                    self.name
                )));
            }
        }
        Ok(())
    }
}

/// 整个会话的配置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionConfig {
    #[serde(default)]
    pub bundles: Vec<BundleSpec>,
}

impl SessionConfig {
    pub fn validate(&self) -> Result<()> {
        let mut bundle_names = Vec::new();
        for b in &self.bundles {
            b.validate()?;
            if !bundle_names.contains(&b.name.as_str()) {
                bundle_names.push(&b.name);
            } else {
                return Err(Error::Config(format!("duplicate bundle name `{}`", b.name)));
            }
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let cfg: SessionConfig = serde_json::from_str(&text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn save(&self, path: &PathBuf) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)? + "\n")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str, port: &str) -> EndpointSpec {
        EndpointSpec {
            name: name.into(),
            kind: EndpointKind::Port { port: port.into() },
        }
    }

    #[test]
    fn split_spec_roundtrips_through_json() {
        let spec = BundleSpec {
            name: "gps-split".into(),
            kind: BundleKind::Split,
            inlets: vec![port("gps", "COM3")],
            outlets: vec![port("app1", "CNCA0"), port("app2", "CNCA1")],
            policy: None,
            acl: vec![AclRule {
                process: "viewer.exe".into(),
                access: "read".into(),
            }],
        };
        let cfg = SessionConfig { bundles: vec![spec] };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: SessionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
        assert_eq!(cfg.bundles[0].effective_policy(), "copy-all");
    }

    #[test]
    fn validation_rejects_short_split() {
        let spec = BundleSpec {
            name: "bad".into(),
            kind: BundleKind::Split,
            inlets: vec![],
            outlets: vec![port("o", "COM4")],
            policy: None,
            acl: vec![],
        };
        assert!(spec.validate().is_err());
    }

    #[test]
    fn validation_rejects_duplicate_endpoint_names() {
        let spec = BundleSpec {
            name: "dup".into(),
            kind: BundleKind::Redirect,
            inlets: vec![port("x", "CNCA0"), port("x", "CNCA1")],
            outlets: vec![],
            policy: None,
            acl: vec![],
        };
        assert!(matches!(spec.validate(), Err(Error::Config(_))));
    }

    #[test]
    fn redirect_requires_two_ends() {
        let ok = BundleSpec {
            name: "pair".into(),
            kind: BundleKind::Redirect,
            inlets: vec![port("a", "COM5")],
            outlets: vec![port("b", "COM6")],
            policy: Some("one-to-one".into()),
            acl: vec![],
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("ccp-core-test");
        let path = dir.join("bundles.json");
        let cfg = SessionConfig {
            bundles: vec![BundleSpec {
                name: "t".into(),
                kind: BundleKind::Share,
                inlets: vec![port("p1", "COM7"), port("p2", "COM8")],
                outlets: vec![port("dev", "COM9")],
                policy: None,
                acl: vec![],
            }],
        };
        cfg.save(&path).unwrap();
        let loaded = SessionConfig::load(&path).unwrap();
        assert_eq!(cfg, loaded);
        let _ = std::fs::remove_file(&path);
    }
}
