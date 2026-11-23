//! 统一错误模型。

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("config: {0}")]
    Config(String),

    #[error("router: {0}")]
    Router(String),

    #[error("setupc: {0}")]
    Setupc(String),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
