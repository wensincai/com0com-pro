//! `ccp-core` — com0com-pro 用户态控制面核心。
//!
//! 职责：
//! - 端点抽象（[`endpoint`]）：串口/命名管道/内存测试端点
//! - bundle 路由引擎（[`router`]）：split / merge / share / redirect
//! - 配置持久化（[`config`]）：bundles.json
//! - 活动日志（[`log`]）：JSONL 审计
//! - setupc 输出解析（[`setupc`]）：端口对列表
//!
//! 本 crate 无 I/O 副作用（日志写入由调用方注入 writer），路由核心纯逻辑可单测。

#![forbid(unsafe_code)]
// GPL-2.0-or-later. com0com-pro is a derivative work of com0com (GPL-2.0).

pub mod config;
pub mod endpoint;
pub mod engine;
pub mod error;
pub mod log;
pub mod router;
pub mod setupc;

pub use config::{
    AclRule, BundleKind, BundleSpec, EndpointKind, EndpointSpec, RoutePolicySpec, SessionConfig,
};
pub use endpoint::{Endpoint, MemoryEndpoint};
pub use engine::BundleEngine;
pub use error::{Error, Result};
pub use log::{ActivityLog, LogEvent};
pub use router::{RoutePolicy, Router};
pub use setupc::{PortPair, parse_pairs_output};
