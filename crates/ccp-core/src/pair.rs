//! PairManager：setupc.exe 子进程封装（端口对安装/删除/改参数/列表）。
//!
//! 设计契约（docs/02-代码分析.md §3.1）：不链接 setup.dll，而是子进程调用
//! `setupc.exe` 并解析 stdout——命令契约自 2012 年起稳定。关键操作幂等。

use crate::error::{Error, Result};
use crate::setupc::{parse_pairs_output, PortPair};
use std::path::{Path, PathBuf};
use std::process::Command;

/// setupc.exe 封装。
#[derive(Debug, Clone)]
pub struct PairManager {
    pub setupc: PathBuf,
}

impl PairManager {
    /// 显式指定 setupc 路径。
    pub fn new(setupc: impl Into<PathBuf>) -> Self {
        Self { setupc: setupc.into() }
    }

    /// 从常见安装路径与 PATH 查找。
    ///
    /// 覆盖三种落点：com0com-pro 安装目录（NSIS 安装器）、旧版 com0com
    /// 目录（官方 3.0 安装包）、本程序同目录与 PATH（绿色版布局）。
    pub fn discover() -> Option<Self> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Ok(dir) = std::env::var(var) {
                let dir = PathBuf::from(dir);
                candidates.push(dir.join("com0com-pro").join("setupc.exe"));
                candidates.push(dir.join("com0com").join("setupc.exe"));
            }
        }
        // 旧版固定路径兜底（环境变量被改过的情况）
        candidates.push(PathBuf::from(r"C:\Program Files (x86)\com0com\setupc.exe"));
        candidates.push(PathBuf::from(r"C:\Program Files\com0com\setupc.exe"));
        // 与本程序同目录（绿色版布局）
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("setupc.exe"));
            }
        }
        // PATH 查找
        if let Ok(path) = std::env::var("PATH") {
            for dir in std::env::split_paths(&path) {
                candidates.push(dir.join("setupc.exe"));
            }
        }
        candidates.into_iter().find(|p| p.exists()).map(Self::new)
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let out = Command::new(&self.setupc)
            .args(args)
            .output()
            .map_err(|e| Error::Setupc(format!("run {}: {e}", self.setupc.display())))?;
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if !out.status.success() {
            return Err(Error::Setupc(format!(
                "setupc {} failed ({}): {}{}",
                args.join(" "),
                out.status,
                stdout.trim(),
                stderr.trim()
            )));
        }
        Ok(stdout)
    }

    /// 列出端口对。
    pub fn list(&self) -> Result<Vec<PortPair>> {
        parse_pairs_output(&self.run(&["list"])?)
    }

    /// 安装一对端口（index 可为 "-" 自动分配）。
    /// 幂等：同编号已存在时报错由调用方先行 list 检查。
    pub fn install(&self, a: &str, b: &str) -> Result<String> {
        self.run(&["install", a, b])
    }

    /// 批量安装（--no-update 加速）后统一更新一次。
    pub fn install_many(&self, pairs: &[(&str, &str)]) -> Result<Vec<String>> {
        let mut outs = Vec::new();
        for (a, b) in pairs {
            outs.push(self.run(&["--no-update", "install", a, b])?);
        }
        if !pairs.is_empty() {
            // 触发一次全量驱动更新
            outs.push(self.run(&["install"])?);
        }
        Ok(outs)
    }

    /// 删除第 n 对端口。
    pub fn remove(&self, index: &str) -> Result<String> {
        self.run(&["remove", index])
    }

    /// 修改端口参数（如 `change CNCA0 EmuBR=yes`）。
    pub fn change(&self, port: &str, param: &str) -> Result<String> {
        self.run(&["change", port, param])
    }

    /// 确保一对端口存在（幂等）：缺失则安装，已有则返回 false。
    pub fn ensure_pair(&self, index: &str, a_params: Option<&str>, b_params: Option<&str>) -> Result<bool> {
        let want_a = format!("CNCA{}", index);
        let want_b = format!("CNCB{}", index);
        let exists = self.list()?.iter().any(|p| p.cnca == want_a && p.cncb == want_b);
        if exists {
            return Ok(false);
        }
        let idx = if index.chars().all(|c| c.is_ascii_digit()) {
            index.to_string()
        } else {
            "-".to_string()
        };
        self.install(&idx, &idx)?;
        if let Some(p) = a_params {
            self.change(&want_a, p)?;
        }
        if let Some(p) = b_params {
            self.change(&want_b, p)?;
        }
        Ok(true)
    }

    /// 删除一对端口（幂等）：不存在时返回 Ok(false)。
    pub fn remove_pair(&self, index: &str) -> Result<bool> {
        let want_a = format!("CNCA{}", index);
        let exists = self.list()?.iter().any(|p| p.cnca == want_a);
        if !exists {
            return Ok(false);
        }
        self.remove(index)?;
        Ok(true)
    }
}

/// 检查 setupc 可用性（服务/GUI 启动前自检）。
pub fn check_available(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_returns_none_without_com0com_installed() {
        // 本机未装 com0com → None 是合法结果（不 panic 即可）
        let _ = PairManager::discover();
    }

    #[test]
    fn ensure_pair_idempotent_without_setupc() {
        // 无 setupc 时应返回 Setupc 错误而非 panic
        let pm = PairManager::new("Z:\\nonexistent\\setupc.exe");
        match pm.ensure_pair("9", None, None) {
            Err(Error::Setupc(_)) => {}
            other => panic!("expected Setupc error, got {other:?}"),
        }
    }
}
