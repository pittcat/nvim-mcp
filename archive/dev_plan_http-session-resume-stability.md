# Dev Plan: HTTP 多客户端 Session 恢复稳定性修复

## 0. 输入前置条件

### 输入前置条件表

| 类别 | 内容 | 是否已提供 | 备注 |
|------|------|------------|------|
| 仓库/模块 | `src/main.rs`、`src/server/core.rs`、`src/server/resources.rs`、`src/server/integration_tests.rs`、`src/test_utils.rs`、`docs/usage.md` | 是 | 已结合现有仓库结构和日志分析报告 |
| 目标接口 | `nvim-mcp` HTTP server、`list_tools`、`connect`、HTTP session resume 路径 | 是 | 主要涉及 HTTP stateful session 与共享连接可见性 |
| 运行环境 | Rust 2024、`rmcp`、Tokio、Claude Code 2.1.76、MCP Protocol 2025-11-25 | 是 | 来源于源码与 `server.log` |
| 约束条件 | 保持 `stdio` 模式可用；不得破坏已有单进程/单客户端行为；不得用弱化断言规避问题 | 是 | 需要兼容现有 `connect` / `list_tools` / HTTP 模式 |
| 已有测试 | `cargo test --lib` 当前通过；已有 HTTP 状态共享回归测试；缺少“多 HTTP client 并发/恢复”测试 | 是 | 当前缺口是并发 HTTP session 覆盖 |
| 需求来源 | `log_analysis_report_20260315_191428.md`、`server.log`、`bug.log`、本次变更请求 | 是 | 根因已在报告中明确 |

### 输入信息处理规则

- 当前信息足以生成执行计划，直接进入计划生成。
- 对于 `rmcp` 内部 session 恢复实现的细节，仓库内不可直接完全观测，因此在计划中以“假设 + 风险”明确标注。
- 如果执行过程中发现根因位于上游 `rmcp` 且仓库内无法无侵入修复，必须将任务升级为 `[BLOCKED]`，并补充最小复现与 workaround 方案。

## 1. 概览（Overview）

- 一句话目标：修复 `nvim-mcp` 在多个 Claude Code 共享 HTTP server 时的 session 恢复异常，消除 `Channel closed: Some(3)` 并稳定多客户端连接行为。
- 优先级：`[P0]`
- 预计时间：1.0 - 1.5 人日
- 当前状态：`[PLANNING]`
- 需求来源：基于 `log_analysis_report_20260315_191428.md`、`server.log`、`bug.log` 与当前仓库实现生成
- 最终交付物：稳定的 HTTP 多客户端 session 恢复修复、自动化回归测试、更新后的使用文档与验证证据

## 2. 背景与目标（Background & Goals）

### 2.1 为什么要做（Why）

当前 `nvim-mcp` 被作为共享 HTTP daemon 使用时，多个 Claude Code 会在短时间内创建多个 HTTP stateful session。日志显示：

- 首个 client 可以成功 `connect` 到目标 Neovim socket
- 后续 client 建立新 session 后，`list_tools` 仍显示 `no connections`
- 更关键的是，服务端在恢复 HTTP session 时触发 `Internal server error when resume session: Session error: Channel closed: Some(3)`

这说明当前 HTTP 共享模式在多客户端并发场景下不稳定。当前痛点包括：

- 共享 daemon 模式无法可靠支撑多个 Claude Code 并发使用
- session 恢复失败会导致内部错误，影响后续请求可靠性
- 陈旧 Neovim socket 会产生 `Connection refused` 噪音，干扰诊断

触发原因来自两个层面：

1. HTTP session 生命周期管理存在不一致，closed session 仍被 resume
2. 连接可见性与 session 范围存在偏差，后续 session 初始看不到已有连接

预期收益：

- 共享 HTTP daemon 可稳定支撑多个 Claude Code 并发连接
- 关键错误从“内部 session 恢复失败”变为“可控、可诊断、可回收”的状态
- 为后续按 workspace 共享一个 `nvim-mcp` 进程提供基础

### 2.2 具体目标（What）

1. 多个 HTTP client 顺序或并发接入同一个 `nvim-mcp` server 时，不再出现 `Channel closed: Some(3)`。
2. 首个 client 建立连接后，后续 session 能以预期方式看到并使用共享连接状态，或以明确的产品语义隔离并稳定工作。
3. 对陈旧 Neovim socket 的连接失败要么被预过滤，要么在日志与返回结果中被明确标记为次级错误，不影响主链路诊断。
4. 新增可自动化执行的 HTTP 多客户端回归测试，覆盖 session create / resume / reconnect 场景。
5. `cargo test --lib`、相关定向测试、构建命令在修复后保持通过。

### 2.3 范围边界、依赖与风险（Out of Scope / Dependencies / Risks）

| 类型 | 内容 | 说明 |
|------|------|------|
| Out of Scope | 重构整个 MCP transport 栈 | 本次只修复当前仓库内可控的 HTTP session / 连接管理问题 |
| Out of Scope | 重写 Claude Code 客户端行为 | 客户端行为只能通过日志推断，不能修改 |
| Out of Scope | 完整替换 `rmcp` 上游 session manager | 仅在本仓库可控范围内做修复或 workaround |
| Dependencies | `rmcp` 的 `StreamableHttpService` / `LocalSessionManager` | 当前 HTTP session 管理依赖上游实现 |
| Dependencies | Neovim socket 生命周期 | 陈旧 socket 需要被识别或清理 |
| Dependencies | 现有 `NeovimMcpServer` 共享状态模型 | 需要保持 stdio / HTTP 两种模式兼容 |
| Risks | 根因位于 `rmcp` 上游，仓库内只能做缓解 | 可能需要补 workaround + 最小复现后上游提 issue |
| Risks | 修复 session 生命周期时破坏现有单客户端 HTTP 行为 | 需要定向回归测试保护 |
| Risks | 修复连接可见性时改变工具暴露语义 | 需要明确 session 共享策略与文档行为 |
| Assumptions | 共享 HTTP daemon 是目标产品语义 | 基于日志报告与现有文档推断 |
| Assumptions | `rmcp` 当前功能可支持多 HTTP client 稳定接入 | 若不成立，需要明确 workaround |
| Assumptions | 自动化测试环境可以构造多 HTTP client 复现路径 | 若不足，需要补最小脚本或 dev-only harness |

### 2.4 成功标准与验收映射（Success Criteria & Verification）

| 目标 | 验证方式 | 类型 | 通过判定 |
|------|----------|------|----------|
| 消除 `Channel closed: Some(3)` 主错误 | 新增多客户端 HTTP 回归测试，例如 `cargo test --lib server::integration_tests::test_http_multi_client_session_resume_stability -- --exact` | 自动 | 测试稳定通过，且日志断言中不包含 `Channel closed: Some(3)` |
| 共享连接状态稳定 | 定向测试，例如 `cargo test --lib server::integration_tests::test_http_multi_client_shared_connection_visibility -- --exact` | 自动 | 第 2/3 个 client 行为符合预期语义且断言通过 |
| 陈旧 socket 行为可控 | 定向测试或日志检查，例如 `cargo test --lib server::integration_tests::test_http_stale_socket_does_not_break_shared_session -- --exact` | 自动 | 连接失败被正确分类，且不会触发 session 内部错误 |
| 不回归现有能力 | `cargo test --lib` | 自动 | 全量 lib 测试通过 |
| 构建可用 | `cargo build` / `cargo build --release` | 自动 | 命令退出码为 0 |
| 文档反映真实行为 | 人工检查 `docs/usage.md` 与 HTTP/stdio 配置说明 | 人工 | 文档明确说明 HTTP 共享 daemon 的启动方式与限制 |

## 3. 技术方案（Technical Design）

### 3.1 高层架构

当前结构：

- Claude Code A / B / C 通过 HTTP 连接同一个 `nvim-mcp`
- `src/main.rs` 创建 `StreamableHttpService`
- `LocalSessionManager` 负责 HTTP stateful session create / resume
- `NeovimMcpServer` 负责连接状态、动态工具、资源与工具调用

修复方向：

1. 固化 HTTP session 生命周期语义，防止 closed session 被继续 resume
2. 明确 session 范围内与全局 server 范围内的连接可见性边界
3. 对陈旧 socket 的 connect 失败做更早识别或更明确隔离

### 3.2 核心流程

1. 启动共享 HTTP server。
2. Client A 创建 session，调用 `connect` 建立 Neovim 连接。
3. Client B / C 创建或恢复 session。
4. 服务端根据 session 状态决定：
   - 是否允许 resume
   - 是否需要清理旧 channel
   - 如何暴露共享连接状态
5. `list_tools` / `connect` / 后续工具调用使用稳定的共享状态。
6. 若 session 已失效，必须显式新建而不是恢复到 closed channel。

ASCII 流程：

```text
HTTP Client -> StreamableHttpService -> LocalSessionManager
           -> create/resume session
           -> NeovimMcpServer(shared state)
           -> connect / list_tools / other tool calls
           -> success OR explicit recoverable error
```

### 3.3 技术栈与运行依赖

- 语言 / 框架：Rust 2024、Tokio
- MCP / HTTP：`rmcp`
- 数据库：无
- 缓存 / 队列 / 中间件：无
- 第三方服务：Neovim RPC / socket
- 构建、测试、部署相关依赖：`cargo build`、`cargo test --lib`、本地 Neovim、HTTP 多客户端测试 harness

### 3.4 关键技术点

- `[CORE]` HTTP session create / resume / cleanup 生命周期的一致性
- `[CORE]` `NeovimMcpServer` 共享状态与 session 范围的关系定义
- `[CORE]` 多 client 下 `list_tools` / `connect` 的预期可见性语义
- `[NOTE]` 集成测试使用预编译二进制；修改影响 `spawn` 行为的代码后需先 `cargo build`
- `[NOTE]` 陈旧 socket 会制造次级噪音，但不能掩盖主错误
- `[OPT]` 增加更细粒度 session 日志，便于后续排障
- `[COMPAT]` 保持 `stdio` 模式与单客户端 HTTP 模式不回归
- `[ROLLBACK]` 若多客户端共享语义修复引发单客户端/stdio 回归，必须回滚到最近通过的基线

### 3.5 模块与文件改动设计

#### 模块级设计

- `src/main.rs`
  - 负责 HTTP server 启动与 session manager 初始化
  - 可能需要调整 HTTP session 管理初始化参数或包装逻辑
- `src/server/core.rs`
  - 负责共享 server 状态与连接表
  - 可能需要增加 session 安全的状态复用或清理逻辑
- `src/server/resources.rs`
  - 负责 `list_tools` / 资源可见性
  - 需要确认多 session 下连接可见性行为
- `src/server/integration_tests.rs`
  - 新增多 HTTP client 并发/恢复/陈旧 socket 回归测试
- `src/test_utils.rs`
  - 增加 HTTP server + 多 client 测试辅助方法
- `docs/usage.md`
  - 更新 HTTP 共享 daemon 的多客户端使用与限制说明

#### 文件级改动清单

| 类型 | 路径 | 说明 |
|------|------|------|
| 修改 | `src/main.rs` | 调整 HTTP session/service 初始化或恢复策略 |
| 修改 | `src/server/core.rs` | 补充共享状态与 session 相关逻辑 |
| 修改 | `src/server/resources.rs` | 校正多 session 下工具/连接可见性表现 |
| 修改 | `src/server/integration_tests.rs` | 增加并发 HTTP session 复现与回归测试 |
| 修改 | `src/test_utils.rs` | 增加 HTTP 多客户端测试辅助工具 |
| 修改 | `docs/usage.md` | 更新使用方式、限制条件、故障规避说明 |
| 可能新增 | `src/server/http_session.rs` | 若需拆分 HTTP session 生命周期逻辑，可新建辅助模块 |
| 可能新增 | `scripts/repro_http_multi_client.sh` | 如自动化测试不足，可补最小复现场景脚本 |

### 3.6 边界情况与异常处理

- 空输入：无有效 Neovim 连接时，`list_tools` 仍应稳定返回而不是异常
- 非法参数：无效 socket path / 非法 URL 不应污染共享 session 状态
- 并发冲突：多个 client 同时 resume / connect 同一 socket 时不得命中 closed channel
- 外部依赖失败：Neovim socket 已失效时应显式报错并可清理
- 兼容旧数据：server 重启后旧 session 不能继续恢复到无效 channel
- 超时 / 重试 / 幂等性：重复 connect 同一 live socket 应保持幂等或有明确定义
- client 中途断开：必须及时清理对应 session/channel，避免幽灵 resume

### 3.7 测试策略

- 单元测试
  - 新增 session 状态转换与失效清理相关单测
  - 新增 stale socket 分类或辅助逻辑单测
- 集成测试
  - 新增多 HTTP client create / resume / reconnect 场景
  - 新增陈旧 socket 与 live socket 混合场景
- 回归测试
  - 保留已有 HTTP 状态共享测试
  - 保持 `connect` / `list_tools` / `stdio` 相关现有测试通过
- lint / build
  - `cargo fmt`
  - `cargo build`
  - `cargo test --lib`
- 必要的人工验证
  - 两个或三个 Claude Code 同时连接共享 HTTP server，观察日志中不再出现 `Channel closed: Some(3)`

## 4. 实施计划（Implementation Plan）

### 4.1 执行基本原则（强制）

1. 所有任务必须可客观验证
2. 任务必须单一目的、可回滚、影响面可控
3. Task N 未验证通过，禁止进入 Task N+1
4. 失败必须记录原因和处理路径，禁止死循环
5. 禁止通过弱化断言、硬编码结果、跳过校验来“伪完成”

### 4.2 分阶段实施

#### 阶段 1：准备与基线确认

- 阶段目标：锁定可重复复现路径，确认当前主错误与次级噪音分离
- 预计时间：2 - 3 小时
- 交付物：可重复的 failing test / repro harness、基线日志
- 进入条件：已有日志报告、仓库可构建
- 完成条件：至少一个自动化或半自动化复现路径稳定命中 `Channel closed: Some(3)`

#### 阶段 2：核心实现

- 阶段目标：修复 HTTP session 恢复生命周期问题，并明确共享连接可见性语义
- 预计时间：4 - 6 小时
- 交付物：核心修复代码、必要的状态管理/清理逻辑
- 进入条件：阶段 1 的 failing reproduction 已建立
- 完成条件：定向修复测试由红转绿

#### 阶段 3：测试与验证

- 阶段目标：通过自动化与人工方式验证多客户端 HTTP 稳定性
- 预计时间：2 - 3 小时
- 交付物：通过的测试结果、日志验证结果
- 进入条件：核心修复已完成
- 完成条件：`cargo test --lib`、关键定向测试、人工并发验证通过

#### 阶段 4：收尾与完成确认

- 阶段目标：更新文档、整理证据、完成交付核对
- 预计时间：1 小时
- 交付物：更新后的文档、最终验证记录
- 进入条件：阶段 3 全部通过
- 完成条件：Definition of Done 全部满足

### 4.3 Task 列表（必须使用统一模板）

#### Task 1: 建立多 HTTP Client 复现基线

| 项目 | 内容 |
|------|------|
| 目标 | 建立稳定可重复的多客户端 HTTP session 复现路径 |
| 代码范围 | `src/server/integration_tests.rs`、`src/test_utils.rs` |
| 预期改动 | 增加 HTTP server 启动辅助与多 client 建链辅助；如必要增加最小脚本 |
| 前置条件 | 当前 `cargo build` 可通过；本地可启动 Neovim |
| 输出产物 | 一个稳定失败的多客户端 HTTP 复现测试或脚本 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo build
cargo test --lib server::integration_tests::test_http_multi_client_session_resume_stability -- --exact --nocapture
```

**通过判定**：

- [PASS] 测试稳定失败，且失败日志包含当前主错误或等价症状

**失败处理**：

- 若测试无法稳定复现，先补日志埋点再重试
- 最多允许 3 次复现方案调整
- 超过 3 次仍不稳定，升级为 `[BLOCKED]` 并保留人工复现脚本

**门禁规则**：

- [BLOCK] 未获得稳定复现前，禁止进入 Task 2

#### Task 2: 补充 HTTP session 生命周期诊断埋点

| 项目 | 内容 |
|------|------|
| 目标 | 让 session create / resume / close / cleanup 路径可观测 |
| 代码范围 | `src/main.rs`、`src/server/core.rs`、可能的 HTTP 辅助模块 |
| 预期改动 | 增加 session_id、resume 结果、cleanup 结果、channel 状态相关日志 |
| 前置条件 | Task 1 已完成 |
| 输出产物 | 更细粒度的调试日志与必要的辅助函数 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo build
cargo test --lib server::integration_tests::test_http_multi_client_session_resume_stability -- --exact --nocapture
```

**通过判定**：

- [PASS] 日志能明确显示 session create / resume / close / cleanup 的完整顺序

**失败处理**：

- 若日志仍不足以解释状态转移，只允许增加最小必要埋点
- 最多 2 次埋点迭代
- 超过阈值后标记 `[BLOCKED]` 并回到输入输出假设

**门禁规则**：

- [BLOCK] 未拿到完整生命周期证据前，禁止进入 Task 3

#### Task 3: 修复 closed session 被 resume 的主链路

| 项目 | 内容 |
|------|------|
| 目标 | 阻止服务端恢复到已关闭的 session/channel |
| 代码范围 | `src/main.rs`、`src/server/core.rs`、可能新增 `src/server/http_session.rs` |
| 预期改动 | 调整 session 恢复策略、失效清理逻辑或 server 侧 workaround |
| 前置条件 | Task 2 已完成 |
| 输出产物 | 主错误修复代码 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo build
cargo test --lib server::integration_tests::test_http_multi_client_session_resume_stability -- --exact --nocapture
```

**通过判定**：

- [PASS] 测试通过，且日志中不再出现 `Channel closed: Some(3)`

**失败处理**：

- 首先回看 session 状态机与 cleanup 顺序
- 最多允许 3 次定向修复
- 超过阈值后标记 `[BLOCKED]`，并区分“仓库内可修”还是“上游 rmcp 问题”

**门禁规则**：

- [BLOCK] 未消除主错误前，禁止进入 Task 4

#### Task 4: 校正多 session 下的共享连接可见性

| 项目 | 内容 |
|------|------|
| 目标 | 明确并修复后续 session 看不到已有连接的行为偏差 |
| 代码范围 | `src/server/resources.rs`、`src/server/core.rs`、`src/server/integration_tests.rs` |
| 预期改动 | 调整 `list_tools` / 连接可见性判定逻辑，或补充显式共享策略 |
| 前置条件 | Task 3 已完成 |
| 输出产物 | 稳定的一致性行为与对应测试 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo build
cargo test --lib server::integration_tests::test_http_multi_client_shared_connection_visibility -- --exact --nocapture
```

**通过判定**：

- [PASS] 第 2/3 个 session 的行为与设计语义一致，测试断言通过

**失败处理**：

- 若行为定义不清，先收敛语义再改代码
- 最多 2 次语义修正
- 超过阈值后标记 `[BLOCKED]`

**门禁规则**：

- [BLOCK] 可见性语义未稳定前，禁止进入 Task 5

#### Task 5: 处理陈旧 socket 次级噪音

| 项目 | 内容 |
|------|------|
| 目标 | 让失效 socket 不再影响主链路诊断或共享 session 稳定性 |
| 代码范围 | `src/server/core.rs`、`src/server/integration_tests.rs` |
| 预期改动 | 增加 stale socket 识别、错误分类或连接前过滤 |
| 前置条件 | Task 4 已完成 |
| 输出产物 | 陈旧 socket 回归测试与更清晰的错误处理 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo build
cargo test --lib server::integration_tests::test_http_stale_socket_does_not_break_shared_session -- --exact --nocapture
```

**通过判定**：

- [PASS] 失效 socket 只产生可控警告或显式错误，不再连带触发 session 内部错误

**失败处理**：

- 若需要外部环境协作，先最小化为可注入的 fake stale socket 场景
- 最多 2 次修复
- 超过阈值后升级为 `[BLOCKED]`

**门禁规则**：

- [BLOCK] 次级噪音未被隔离前，禁止进入 Task 6

#### Task 6: 扩充并稳定自动化测试矩阵

| 项目 | 内容 |
|------|------|
| 目标 | 将本次问题固化为回归测试矩阵 |
| 代码范围 | `src/server/integration_tests.rs`、`src/test_utils.rs` |
| 预期改动 | 增加并发 HTTP tests、必要的测试 helper、清理逻辑 |
| 前置条件 | Task 3-5 已完成 |
| 输出产物 | 可重复执行的自动化回归测试 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo build
cargo test --lib
```

**通过判定**：

- [PASS] `cargo test --lib` 全量通过，新增测试稳定无偶发失败

**失败处理**：

- 先定位 flakiness 是否来自 session、socket、还是测试清理
- 最多 3 次稳定性修复
- 超过阈值后标记 `[BLOCKED]`

**门禁规则**：

- [BLOCK] 全量 lib 测试未通过前，禁止进入 Task 7

#### Task 7: 完成文档与人工并发验证

| 项目 | 内容 |
|------|------|
| 目标 | 更新使用文档并完成真实多 Claude Code 场景验证 |
| 代码范围 | `docs/usage.md`，必要时补充报告/说明文档 |
| 预期改动 | 更新 HTTP daemon 多客户端说明、限制、启动方式、已知行为 |
| 前置条件 | Task 6 已完成 |
| 输出产物 | 更新后的文档与人工验证记录 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo build --release
./target/release/nvim-mcp --http-port 8080 --connect auto --log-file ./server.log --log-level debug
```

人工检查点：

- 同时启动 2-3 个 Claude Code 连接到同一个 HTTP server
- 执行 `list_tools` / `connect` / 至少一个 connection-aware tool
- 检查 `server.log` 中不存在 `Channel closed: Some(3)`

**通过判定**：

- [PASS] 人工并发验证通过，文档已与真实行为一致

**失败处理**：

- 先保留日志并回退到最近一个通过的自动化基线
- 最多 2 次人工复现修正
- 超过阈值后标记 `[BLOCKED]`

**门禁规则**：

- [BLOCK] 人工验证未通过前，禁止标记整体完成

## 5. 失败处理协议（Error-Handling Protocol）

| 级别 | 触发条件 | 处理策略 |
|------|----------|----------|
| Level 1 | 单次验证失败 | 原地修复，禁止扩大重构 |
| Level 2 | 连续 3 次失败 | 回到假设和接口定义，重新核对输入输出 |
| Level 3 | 仍无法通过 | 停止执行，记录 Blocker，等待人工确认 |

### 重试规则

- 每次修复必须记录变更范围
- 每次重试前必须更新状态
- 同一类失败不得无限重复
- 达到阈值必须升级，不得原地空转

## 6. 状态同步机制（Stateful Plan）

### 状态标记规范

| 标记 | 含义 |
|------|------|
| [TODO] | 未开始 |
| [DOING] | 进行中 |
| [DONE] | 已完成且验证通过 |
| [BLOCKED] | 阻塞 |
| [PASS] | 当前验证通过 |
| [FAIL] | 当前验证失败 |

### 强制要求

- 每一轮执行必须更新状态
- 未验证通过前禁止标记 `[DONE]`
- 遇到问题必须记录失败原因和阻塞点
- 若阶段完成，必须同步更新阶段状态

## 7. Anti-Patterns（禁止行为）

- `[FORBIDDEN]` 禁止删除或弱化现有断言
- `[FORBIDDEN]` 禁止为了通过测试而硬编码返回值
- `[FORBIDDEN]` 禁止跳过验证步骤
- `[FORBIDDEN]` 禁止引入未声明依赖
- `[FORBIDDEN]` 禁止关闭 lint / typecheck / 类型检查以规避问题
- `[FORBIDDEN]` 禁止修改超出范围的模块
- `[FORBIDDEN]` 禁止在未记录原因的情况下扩大重构范围

违反后的动作：

- Task 标记为 `[BLOCKED]`
- 必须回滚到最近一个验证通过点
- 必须记录触发原因

## 8. 最终完成条件（Definition of Done）

- 所有计划内 Task 已完成
- 所有关键验证已通过
- 没有未记录的 blocker
- 约束条件仍被满足
- 交付物已齐备
- 成功标准与验收映射表中的项目全部完成

## 9. 质量检查清单

- [ ] 所有目标都有验证方式
- [ ] 所有 Task 都有验证方式
- [ ] 所有 Task 都具备原子性和可回滚性
- [ ] 已明确 Out of Scope
- [ ] 已明确依赖与风险
- [ ] 已明确文件级改动范围
- [ ] 已定义失败处理协议
- [ ] 已定义 Anti-Patterns
- [ ] 已定义最终完成条件
- [ ] 当前 Plan 可被 Agent 连续执行
- [ ] 当前结构可转换为 Ralph Spec
