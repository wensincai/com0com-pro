//! `setupc.exe` 输出解析与调用封装（契约层）。
//!
//! PairManager 不链接 setup.dll，而是子进程调用 `setupc.exe` 并解析其
//! stdout——`setupc` 的命令契约自 2012 年起稳定，且有 `list` 输出可做
//! 回归测试。这里只包含纯解析（可单测）；进程调用在 P3 的引擎宿主。

use crate::error::{Error, Result};

/// 一对端口（CNCAx / CNCBx）及其参数串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortPair {
    pub cnca: String,
    pub cnca_params: String,
    pub cncb: String,
    pub cncb_params: String,
}

impl PortPair {
    /// 从参数串提取 PortName（`-` 表示未命名）。
    pub fn port_name(params: &str) -> Option<String> {
        for part in params.split(',') {
            if let Some(v) = part.strip_prefix("PortName=") {
                if v == "-" {
                    return None;
                }
                return Some(v.to_string());
            }
        }
        None
    }
}

/// 解析 `setupc list` 的输出。
///
/// 典型输出（缩进可能不一）：
/// ```text
///     CNCA0 PortName=COM#,RealPortName=COM3
///     CNCB0 PortName=-
///     CNCA1 EmuBR=yes
///     CNCB1 PortName=COM9,EmuOverrun=yes
/// ```
pub fn parse_pairs_output(text: &str) -> Result<Vec<PortPair>> {
    let mut pairs = Vec::new();
    let mut pending: Option<(String, String)> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let (name, params) = line
            .split_once(char::is_whitespace)
            .map(|(n, p)| (n.to_string(), p.trim().to_string()))
            .unwrap_or_else(|| (line.to_string(), String::new()));

        if !name.starts_with("CNCA") && !name.starts_with("CNCB") {
            return Err(Error::Setupc(format!("unexpected line in list output: {raw:?}")));
        }
        if name.starts_with("CNCA") {
            if pending.is_some() {
                return Err(Error::Setupc("consecutive CNCA lines".into()));
            }
            pending = Some((name, params));
        } else {
            let (cnca, cnca_params) =
                pending.take().ok_or_else(|| Error::Setupc("CNCB without CNCA".into()))?;
            // 编号必须配对
            if cnca[4..] != name[4..] {
                return Err(Error::Setupc(format!("pair mismatch: {cnca} / {name}")));
            }
            pairs.push(PortPair {
                cnca,
                cnca_params,
                cncb: name,
                cncb_params: params,
            });
        }
    }
    if pending.is_some() {
        return Err(Error::Setupc("dangling CNCA without CNCB".into()));
    }
    Ok(pairs)
}

/// 构造 `setupc install <a> <b>` 风格参数（供 P3 进程封装）。
pub fn install_args(a: &str, b: &str) -> Vec<String> {
    vec!["install".into(), a.into(), b.into()]
}

/// 构造 `setupc change <port> <param>` 参数。
pub fn change_args(port: &str, param: &str) -> Vec<String> {
    vec!["change".into(), port.into(), param.into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
    CNCA0 PortName=COM#,RealPortName=COM3
    CNCB0 PortName=-
    CNCA1 EmuBR=yes
    CNCB1 PortName=COM9,EmuOverrun=yes
";

    #[test]
    fn parses_wellformed_list_output() {
        let pairs = parse_pairs_output(SAMPLE).unwrap();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].cnca, "CNCA0");
        assert_eq!(pairs[0].cnca_params, "PortName=COM#,RealPortName=COM3");
        assert_eq!(pairs[0].cncb_params, "PortName=-");
        assert_eq!(pairs[1].cncb_params, "PortName=COM9,EmuOverrun=yes");
    }

    #[test]
    fn port_name_extracts_or_none() {
        assert_eq!(PortPair::port_name("PortName=COM9,EmuBR=yes"), Some("COM9".into()));
        assert_eq!(PortPair::port_name("PortName=-"), None);
        assert_eq!(PortPair::port_name("EmuBR=yes"), None);
    }

    #[test]
    fn rejects_unpaired_lines() {
        assert!(parse_pairs_output("CNCA0 PortName=-").is_err());
        assert!(parse_pairs_output("CNCB0 PortName=-").is_err());
        assert!(parse_pairs_output("CNCA0 PortName=-\nCNCA1 PortName=-\nCNCB0 PortName=-").is_err());
    }

    #[test]
    fn rejects_number_mismatch() {
        let bad = "CNCA0 PortName=-\nCNCB1 PortName=-";
        assert!(parse_pairs_output(bad).is_err());
    }

    #[test]
    fn rejects_foreign_lines() {
        assert!(parse_pairs_output("garbage line").is_err());
    }

    #[test]
    fn arg_builders() {
        assert_eq!(install_args("0", "-"), vec!["install", "0", "-"]);
        assert_eq!(change_args("CNCA0", "EmuBR=yes"), vec!["change", "CNCA0", "EmuBR=yes"]);
    }
}
