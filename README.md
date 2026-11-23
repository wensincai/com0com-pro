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

## 当前状态（P2 完成）

- **ccp-core**（库）：端点抽象（`Endpoint` trait + 内存测试端点）、bundle 路由器
  （`OneToOne` / `CopyAll` / `RoundRobin` / `FirstWriterWins` 四种策略）、
  配置模型（bundles.json）、JSONL 活动日志、setupc 输出解析器
- **ccp-cli**（`ccp.exe`）：`init`（生成示例配置）/ `validate` / `list`（解析本机 setupc 端口对）
- **测试**：`cargo test --workspace` 29 项全绿；`cargo clippy` 0 警告
- 基础驱动代码：`E:\reverseEngineering\20261112\com0com`（VS2022+WDK 迁移完成，测试签名通过，见其 MIGRATION-REPORT.md）

## 构建

```bat
cargo build --release        :: target\release\ccp.exe
cargo test --workspace       :: 29 项单测（内存端点，无需真实串口）
```

驱动构建：在 com0com 目录执行 `build.cmd`（需要 VS2022 + WDK 10.0.26100）。

## 快速体验

```bat
ccp init sample-bundles.json          :: 生成示例（gps-split、tool-link）
ccp validate sample-bundles.json      :: 校验拓扑
ccp list                              :: 列出本机 com0com 端口对（需装 com0com）
```

## 里程碑

- [x] **P1** 文档与选型（本仓库 docs/）
- [x] **P2** bundle 引擎核心（ccp-core：路由器/配置/日志/解析器 + 29 项单测）
- [ ] **P3** CLI 实机运行：SerialEndpoint（serialport crate）+ PairManager（调 setupc.exe）+ bundle 实跑
- [ ] **P4** Windows 服务（开机重建）+ egui 管理界面
- [ ] **P5** 驱动小改：强删已打开端口、按会话端口

## 许可证

GPL-2.0-or-later（见 [LICENSE](LICENSE)）。本项目是 com0com（作者 Vyacheslav Frolov）的衍生作品，com0com 部分版权归其作者所有。
