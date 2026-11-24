# com0com-pro

**基于 com0com 的增强版虚拟串口套件** —— 保留久经使用的开源 com0com 内核驱动（GPL-2.0），在其上新建用户态控制面，补齐商业 VSPD（Eltima Virtual Serial Port Driver）的功能差距。

## 定位

| | com0com 3.0（基础） | VSPD 11（商业对标） | **com0com-pro（本项目）** |
|---|---|---|---|
| 虚拟串口对 / 引脚定制 / 波特率仿真 / 噪声注入 | ✅ | 部分 | ✅（复用 com0com.sys） |
| Split / Merge / Share / Redirect 端口捆绑 | ❌ | ✅ | 🚧 开发中（P2 引擎核心已完成） |
| 活动日志 / 每端口 ACL | ❌ | ✅ | 🚧 |
| 编程 API / 现代 GUI / 服务自启 | 弱 | ✅ | 🚧 |

## 工程文档

| 文档 | 内容 |
|---|---|
| [docs/01-需求分析.md](docs/01-需求分析.md) | com0com 与 VSPD 功能差距矩阵、FR/NFR、里程碑 |
| [docs/02-代码分析.md](docs/02-代码分析.md) | com0com 源码结构、复用/改造/新建决策 |
| [docs/03-技术选型.md](docs/03-技术选型.md) | 驱动保持 C/WDM、控制面选 Rust 的论证 |
| [docs/04-架构设计.md](docs/04-架构设计.md) | 控制面/数据面分离、bundle 拓扑模型、测试策略 |
| [docs/05-驱动增强说明.md](docs/05-驱动增强说明.md) | P5 强删已打开端口：机制、开启方法、风险与 VM 验证计划 |
| **[docs/06-使用指南.md](docs/06-使用指南.md)** | **新手从这里开始**：安装步骤、GUI/CLI 用法、三种典型场景、故障排查 |

## 当前状态（P1–P6 完成）

| 里程碑 | 内容 | 状态 |
|---|---|---|
| P1 | 需求分析 / 代码分析 / 技术选型 / 架构设计 | ✅ |
| P2 | ccp-core 路由引擎（四策略）+ 配置 + 日志 + setupc 解析 | ✅ 36 项单测 |
| P3 | 实机端点（串口/TCP）+ 泵线程宿主 + PairManager + CLI serve/pair | ✅ |
| P4 | ccp-service（SCM 服务）+ ccp-gui（egui 管理界面） | ✅ 可构建 |
| P5 | 驱动强删已打开端口（默认关闭，需 VM 验证） | ✅ 编译通过 |
| P6 | NSIS 安装包（驱动+工具+服务+默认端口对） | ✅ |

- **4 个 crate**：`ccp-core`（库）、`ccp-cli`（ccp.exe）、`ccp-service`（Windows 服务）、`ccp-gui`（egui）
- **质量**：`cargo test --workspace` 36 项绿 / 1 项手动 ignored；`cargo clippy` 0 警告
- **驱动**：com0com 目录 `build.cmd` 通过（VS2022 + WDK 10.0.26100），含 P5 改动

## 构建

```bat
cargo build --release        :: target\release\{ccp,ccp-service,ccp-gui}.exe
cargo test --workspace       :: 36 项单测（内存设备，无需真实串口/驱动）

:: NSIS 安装包（需 NSIS 3）
"C:\Program Files (x86)\NSIS\makensis.exe" installer\com0com-pro.nsi
:: 产物：installer\com0com-pro-setup.exe
```

## 快速体验

```bat
ccp init bundles.json                 :: 生成示例（split + redirect）
ccp validate bundles.json             :: 校验拓扑
ccp ports                             :: 枚举本机串口
ccp pair list                         :: 列出 com0com 端口对（需装 com0com）
ccp pair ensure 0 --a EmuBR=yes --b EmuOverrun=yes   :: 幂等创建端口对
ccp serve bundles.json                :: 实机运行全部 bundle，Ctrl+C 停止
ccp service install                   :: 安装开机自启服务（管理员）
ccp-gui                               :: 图形界面：bundle 编辑 / 端口 / 活动日志
```

配置文件格式（`bundles.json`）：

```json
{
  "bundles": [
    {
      "name": "gps-split",
      "kind": "split",
      "inlets":  [{ "name": "gps", "type": "port", "port": "COM3" }],
      "outlets": [{ "name": "nav", "type": "port", "port": "CNCA0" },
                  { "name": "log", "type": "port", "port": "CNCA1" }]
    }
  ]
}
```

`kind`：`split` / `merge` / `share` / `redirect` / `complex`；端点 `type`：`port` / `tcp` / `pipe`（pipe 待 P6）。
策略缺省：split=copy-all、merge=round-robin、share=first-writer-wins:1000、redirect=one-to-one。

## 里程碑

- [x] **P1** 文档与选型
- [x] **P2** bundle 引擎核心
- [x] **P3** 实机运行（串口/TCP 端点、PairManager、CLI）
- [x] **P4** Windows 服务 + egui 管理界面
- [x] **P5** 驱动强删已打开端口（默认关闭，待 VM 验证）
- [ ] **P6** 命名管道端点与 JSON-RPC API、按会话端口、NSIS 打包、性能压测

## 许可证

GPL-2.0-or-later（见 [LICENSE](LICENSE)）。本项目是 com0com（作者 Vyacheslav Frolov）的衍生作品，com0com 部分版权归其作者所有。
