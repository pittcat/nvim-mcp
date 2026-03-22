# Dev Plan: HTTP Session 与连接稳健性优化

## 输入前置条件

### 输入前置条件表

| 类别 | 内容 | 是否已提供 | 备注 |
|------|------|------------|------|
| 仓库/模块 | `src/main.rs`、`src/logging.rs`、`src/server/{core,tools,resources,integration_tests.rs}`、`src/neovim/{client,connection}.rs`、`docs/{tools,usage}.md` | 是 | 已根据仓库结构和报告定位主要改动范围 |
| 目标接口 | `/mcp` Streamable HTTP、session/resume、`connect`、`connect_tcp`、`disconnect`、`get_targets`、`exec_lua`、`nvim-connections://`、`nvim-tools://` | 是 | 来自报告与源码检索 |
| 运行环境 | Rust 2024、`rmcp=0.14`、`nvim-rs=0.9.2`、`tracing=0.1.44`、Neovim socket/TCP | 是 | 版本来自报告与仓库上下文 |
| 约束条件 | 不实现客户端侧 Lua 生成策略修正；保持多客户端共享 daemon 语义；不破坏既有 MCP tool 接口 | 是 | 用户已明确排除客户端 Lua 生成策略 |
| 已有测试 | `src/server/integration_tests.rs` 已覆盖多 session / stale socket；`src/neovim/integration_tests.rs` 已覆盖基础连接执行路径 | 是 | 缺少 closed-channel resume 降级、连接收敛、`connect_tcp` 误用提示回归 |
| 需求来源 | `log_analysis_report_20260316_224914.md` + 用户口头变更请求 | 是 | 本计划基于当前仓库可控范围生成 |

### 输入信息处理规则

- 已知信息：
  - 报告共给出 7 个优化建议。
  - 用户要求排除“客户端侧 Lua 生成策略修正”，实现其余优化项。
- 缺失信息：
  - 外部 MCP 客户端代码与发布节奏未提供。
  - `rmcp` 对 closed-channel resume 的内部错误分类能力需以实现时验证。
- 当前假设：
  - 第 2 条“客户端收到 `Channel closed` 后停止复用旧 resume 路径”不属于本仓库直接可修改范围。
  - 本仓库可落地的对应优化是：服务端返回可恢复错误、文档明确恢复动作、日志可诊断、测试覆盖。
- 风险约束：
  - 禁止把外部客户端行为写成已可在本仓库内直接完成的确定事实。
  - 若实现时发现 `rmcp` 错误分类能力不足，需在风险中升级标记。

## 1. 概览（Overview）

- 一句话目标：在不修改客户端 Lua 生成策略的前提下，完成 `nvim-mcp` 服务端侧 HTTP session 恢复、连接生命周期、错误可诊断性、长连接观测与 `connect_tcp` 误用提示优化。
- 优先级：`[P0]`
- 预计时间：10-14 小时
- 当前状态：`[PLANNING]`
- 需求来源：`log_analysis_report_20260316_224914.md` 中除“客户端侧 Lua 生成策略”外的优化建议，以及用户追加要求
- 最终交付物：服务端实现变更、回归测试、文档更新、可执行验收命令集

## 2. 背景与目标（Background & Goals）

### 2.1 为什么要做（Why）

当前主链路 `nvim-mcp -> Neovim RPC` 并未整体失效，报告已证明大部分 `get_targets -> connect -> exec_lua -> navigate` 调用可成功。问题集中在以下几类：

- `exec_lua` 下游失败后，HTTP resume 命中已关闭 channel，导致 `500 Internal Server Error`
- reconnect / disconnect / 连接失效后，旧 `connection_id` 继续被暴露或复用，产生 `Not connected`、`Unknown`
- `/mcp` 长连接提前结束时，日志上下文不足，定位依赖跨文件人工拼接
- `connect_tcp` 被误传 Unix socket 路径时，失败是预期内的，但提示不够直接
- closed-channel resume 被表现成“服务端内部崩溃”，误导问题判断

当前痛点：

- 错误语义不清
- 状态收敛不足
- 诊断路径成本高
- 文档与错误提示不够明确

触发原因：

- session 生命周期异常未被降级为业务可恢复错误
- 共享连接池缺少 stale connection 收敛机制
- 请求、session、stream、connection 四类上下文未统一关联
- 输入误用前置校验不足

预期收益：

- 消除 closed-channel resume 导致的 HTTP 500 误报
- 降低 `Not connected` / `Unknown` 状态漂移噪音
- 让单次问题在一份日志中可直接定位
- 为外部客户端提供稳定、明确、可恢复的错误契约
- 减少 `connect_tcp` 的低价值误用噪音

### 2.2 具体目标（What）

1. 将 closed-channel resume 的响应从 HTTP 500 降级为明确、可恢复的错误结果，并提示重新初始化 session。
2. 为 `connection_id` 建立更稳健的生命周期收敛逻辑，避免失效连接继续以“可用连接”身份暴露。
3. 为 HTTP 请求、tool 调用、session、connection、stream 生命周期补齐统一日志上下文。
4. 为 `/mcp` 长连接记录客户端类型、连接持续时间、关闭原因等健康观测信息。
5. 为 `connect_tcp` 增加 Unix socket 路径误用前置校验，明确提示改用 `connect`。
6. 保持既有多客户端共享 daemon 语义、工具接口和现有核心测试不回退。
7. 更新用户文档，使新的错误契约、恢复动作与排障方式可被直接理解和执行。

### 2.3 范围边界、依赖与风险（Out of Scope / Dependencies / Risks）

| 类型 | 内容 | 说明 |
|------|------|------|
| Out of Scope | 客户端侧 Lua 生成策略修正 | 用户明确排除，不在本次计划内 |
| Out of Scope | 外部 Claude Code / 第三方 MCP client 的 resume 重试逻辑修改 | 不在当前仓库所有权范围内 |
| Dependencies | `rmcp` Streamable HTTP 与 local session manager 行为 | closed-channel resume 的错误映射依赖其可拦截性 |
| Dependencies | `nvim-rs` 连接与 `io_handler` 生命周期 | stale connection 清理需基于底层连接状态与失败信号 |
| Dependencies | 现有多客户端共享连接语义 | 必须保持跨 session 共享同一连接池的行为 |
| Risks | `rmcp` 可能无法细粒度区分所有 session 恢复失败类型 | 可能需要在 `src/main.rs` HTTP 边界做错误包装 |
| Risks | 连接清理过于激进可能误伤共享连接 | 需要测试覆盖 reconnect / disconnect / stale 场景 |
| Risks | 外部客户端若继续无条件复用旧 session，仍会重复失败 | 本次只能做到服务端可恢复错误与文档提示 |
| Risks | 日志字段过多可能增加噪音和 I/O 成本 | 需要字段裁剪与统一格式 |
| Assumptions | `Channel closed` 场景可稳定映射为“需要重建 session”的可恢复错误 | 基于当前报告与已有日志模式 |
| Assumptions | 失效连接可通过调用失败、断开状态或 `io_handler` 生命周期推断 | 实现时需验证是否足够可靠 |
| Assumptions | 现有 `src/server/integration_tests.rs` 可承载新增回归场景 | 优先复用既有测试基座 |

### 2.4 成功标准与验收映射（Success Criteria & Verification）

| 目标 | 验证方式 | 类型 | 通过判定 |
|------|----------|------|----------|
| closed-channel resume 不再返回 500 | `cargo test test_http_closed_channel_resume_returns_recoverable_error -- --nocapture` | 自动 | 响应状态不为 500，返回体含可恢复错误语义与重建 session 提示 |
| 连接生命周期收敛 | `cargo test test_http_stale_connection_is_pruned_from_registry -- --nocapture` | 自动 | stale 连接不再继续以有效连接身份暴露或被错误复用 |
| 多客户端共享语义保持稳定 | `cargo test test_http_multi_client_session_resume_stability -- --nocapture`、`cargo test test_http_multi_client_shared_connection_visibility -- --nocapture` | 自动 | 既有共享连接与多 session 行为保持通过 |
| 日志关联信息完整 | 人工检查 `debug_log.txt` 中单次失败链路 | 人工 | 同一故障链路中可看到 `session_id`、`request_id`、`toolUseId`、`connection_id`、关闭原因/持续时长 |
| 长连接健康观测落地 | 启动 HTTP server 并手工复现一次连接建立与提前关闭 | 人工 | 日志中存在客户端类型、持续时间、关闭原因 |
| `connect_tcp` 误用提示清晰 | `cargo test test_connect_tcp_rejects_unix_socket_path_with_hint -- --nocapture` | 自动 | 返回错误明确提示“Unix socket 请使用 connect”或等价语义 |
| 全量回归稳定 | `./scripts/run-test.sh -- --show-output` | 自动 | 仓库测试通过，无新增 `Channel closed` / 500 回归 |
| 文档与实现一致 | `pre-commit run --all-files` + 文档人工核对 | 自动/人工 | 文档中描述的恢复方式、错误语义与实现一致 |

## 3. 技术方案（Technical Design）

### 3.1 高层架构

```mermaid
flowchart LR
    A[MCP Client] --> B[/mcp HTTP + SSE<br/>src/main.rs]
    B --> C[StreamableHttpService<br/>LocalSessionManager]
    C --> D[NeovimMcpServer<br/>src/server/core.rs]
    D --> E[Tool/Resource Routing<br/>src/server/{resources,tools}.rs]
    E --> F[Connection Registry<br/>DashMap<connection_id, client>]
    F --> G[NeovimClient / NeovimConnection<br/>src/neovim/{client,connection}.rs]
    B --> H[HTTP Error Normalizer<br/>closed-channel -> recoverable error]
    B --> I[Stream Health Logging<br/>duration / close reason / client type]
    D --> J[Shared Connection Semantics]
    E --> K[Input Guard<br/>connect_tcp misuse check]
    B --> L[Structured Logging<br/>src/logging.rs]
```

结构说明：

- `src/main.rs` 负责 HTTP transport、resume 错误降级、连接级日志与 stream 生命周期观测。
- `src/server/core.rs` 维护共享连接池与连接资源展示，需要收敛 stale connection 状态。
- `src/server/tools.rs` 负责 `connect` / `connect_tcp` / `disconnect` / `exec_lua` 等工具入口，需要增加前置校验与错误语义统一。
- `src/server/resources.rs` 是请求与工具分发日志入口，需要补齐请求上下文。
- `src/neovim/client.rs` 与 `src/neovim/connection.rs` 负责底层连接生命周期信息。
- `src/logging.rs` 负责统一日志上下文提取与输出格式。

### 3.2 核心流程

1. 客户端通过 `/mcp` 初始化 session，服务端分配 `mcp-session-id`。
2. 客户端发起工具调用，`ServerHandler::call_tool` 记录请求上下文并分发。
3. 工具调用涉及连接查询时：
   - 若连接有效，正常执行
   - 若连接已失效，返回明确连接错误并触发注册表收敛
4. 当 HTTP resume 命中已关闭 channel 时：
   - 在 HTTP service 边界识别 closed-channel 类错误
   - 返回可恢复错误，而不是 HTTP 500
   - 提示重新初始化 session
5. 长连接建立与关闭时：
   - 记录客户端类型、session_id、持续时间、关闭原因
6. `connect_tcp` 收到 Unix socket 路径时：
   - 工具层前置拒绝
   - 返回“请使用 connect”的纠错提示

### 3.3 技术栈与运行依赖

- 语言 / 框架：
  - Rust 2024
  - Tokio
  - Hyper / Tower
  - Clap
- 数据库：
  - 无
- 缓存 / 队列 / 中间件：
  - 无
  - 使用 `rmcp` local session manager 管理 HTTP session
- 第三方服务：
  - Neovim RPC（`nvim-rs`）
  - MCP transport（`rmcp`）
- 构建、测试、部署相关依赖：
  - `cargo build`
  - `./scripts/run-test.sh -- --show-output`
  - `pre-commit run --all-files`
  - `nix develop .`

### 3.4 关键技术点

- `[CORE]` 在不破坏现有 stateful HTTP 行为的前提下，将 closed-channel resume 归类为“可恢复错误”。
- `[CORE]` 为共享连接池增加 stale connection 收敛逻辑，避免失效连接继续被资源层与工具层当作有效连接。
- `[CORE]` 在少数公共入口统一请求、session、connection、stream 的日志关联字段。
- `[NOTE]` 第 2 条优化建议中的“客户端停止复用旧 resume 路径”不属于本仓库直接改动范围，本次只做服务端契约与文档配套。
- `[OPT]` 对 `/mcp` 长连接增加健康观测日志，降低后续排障成本。
- `[OPT]` 对 `connect_tcp` 做输入类型前置校验，减少无效错误噪音。
- `[COMPAT]` 必须保持多客户端共享同一连接池、跨 session 可见连接 ID 的现有语义。
- `[COMPAT]` 不修改既有 MCP tool 名称、主参数结构与核心成功路径。
- `[ROLLBACK]` 若 closed-channel 错误映射引发兼容问题，可单独回滚 HTTP error normalizer。
- `[ROLLBACK]` 若连接收敛策略影响共享连接可见性，可单独回滚连接清理逻辑，保留日志增强与输入校验。

### 3.5 模块与文件改动设计

#### 模块级设计

- HTTP Transport：
  - 在 `src/main.rs` 增加 resume 错误降级、stream 生命周期日志、连接关闭原因记录。
- Logging：
  - 在 `src/logging.rs` 增强 request/session/connection/stream 关联字段提取。
- Tool Routing：
  - 在 `src/server/resources.rs` 与 `src/server/tools.rs` 统一错误上下文与输入校验。
- Connection Registry：
  - 在 `src/server/core.rs` 约束共享连接池中的 stale entry 展示与使用。
- Neovim Client：
  - 在 `src/neovim/client.rs` / `src/neovim/connection.rs` 暴露可用于连接状态判断的辅助信息。
- Testing：
  - 在 `src/server/integration_tests.rs` 增加 closed-channel resume、连接收敛、`connect_tcp` 提示回归。
- Documentation：
  - 在 `docs/tools.md`、`docs/usage.md` 更新工具边界、恢复语义和排障说明。

#### 文件级改动清单

| 类型 | 路径 | 说明 |
|------|------|------|
| 新增 | 无 | 优先复用既有模块与测试文件 |
| 修改 | `src/main.rs` | HTTP error normalizer、stream 健康观测日志、session/resume 边界处理 |
| 修改 | `src/logging.rs` | 统一日志上下文字段提取与裁剪 |
| 修改 | `src/server/core.rs` | 共享连接池的 stale connection 收敛与资源展示稳定化 |
| 修改 | `src/server/tools.rs` | `connect_tcp` 误用前置校验、连接相关错误语义统一 |
| 修改 | `src/server/resources.rs` | request/tool 级日志上下文补强 |
| 修改 | `src/neovim/client.rs` | 连接失效识别与生命周期辅助信息 |
| 修改 | `src/neovim/connection.rs` | 连接生命周期元信息支持 |
| 修改 | `src/server/integration_tests.rs` | 新增和修改 HTTP/session/connection 回归测试 |
| 修改 | `docs/tools.md` | `connect` / `connect_tcp` 使用边界和错误说明 |
| 修改 | `docs/usage.md` | multi-client HTTP 模式的恢复语义和排障更新 |
| 删除 | 无 | 本次不涉及删除文件 |

### 3.6 边界情况与异常处理

- 空或缺失 `mcp-session-id`
- session 已失效或 resume 命中 closed channel
- `connection_id` 存在但底层连接已失效
- reconnect 到相同 target 时旧连接清理不完整
- 多客户端共享同一连接时其中一方执行 `disconnect`
- `connect_tcp` 误传 Unix socket 路径
- 真实 TCP 地址不能被误判为 socket 路径
- `/mcp` 长连接提前关闭但无明确上游错误体
- 日志字段过长导致可读性下降
- 既有共享连接资源展示逻辑不能因收敛策略被破坏
- stale socket 错误不能再次被转化为 `Channel closed`
- 人工复现场景下必须能区分“工具错误”“session 错误”“连接错误”

### 3.7 测试策略

单元测试：

- 新增日志上下文提取与错误分类相关测试
- 新增地址类型判断测试
- 必要时为连接状态辅助逻辑增加轻量单测

集成测试：

- 新增 closed-channel resume 降级测试
- 新增 stale connection 清理测试
- 新增 `connect_tcp` Unix socket 误用提示测试

回归测试：

- 必须保持通过：
  - `test_http_multi_client_session_resume_stability`
  - `test_http_multi_client_shared_connection_visibility`
  - `test_http_stale_socket_does_not_break_shared_session`

lint / build：

- `cargo build`
- `./scripts/run-test.sh -- --show-output`
- `pre-commit run --all-files`

必要的人工验证：

- 启动 HTTP server
- 复现一次失败 `exec_lua` 后的 session 恢复路径
- 确认不再出现 HTTP 500
- 检查 `debug_log.txt` 是否能直接串起完整问题链路

新增测试：

- closed-channel resume 降级测试
- stale connection 从共享连接池收敛测试
- `connect_tcp` 误用前置提示测试
- 日志上下文完整性测试

修改测试：

- 根据新错误语义调整部分错误断言
- 必要时扩展现有 HTTP 集成测试的响应断言

必须保持通过的现有测试：

- 所有多客户端共享连接相关测试
- 所有 stale socket 不破坏 session 的测试
- 基础 Neovim 连接与 `exec_lua` 成功路径测试

## 4. 实施计划（Implementation Plan）

### 4.1 执行基本原则（强制）

1. 所有任务必须可客观验证
2. 任务必须单一目的、可回滚、影响面可控
3. Task N 未验证通过，禁止进入 Task N+1
4. 失败必须记录原因和处理路径，禁止死循环
5. 禁止通过弱化断言、硬编码结果、跳过校验来“伪完成”

### 4.2 分阶段实施

#### 阶段 ID 与执行顺序

| 阶段 ID | 阶段名称 | 依赖 | 状态 |
|------|------|------|------|
| `S1` | 准备与基线确认 | 无 | `[TODO]` |
| `S2` | 核心实现 | `S1` | `[TODO]` |
| `S3` | 测试与验证 | `S2` | `[TODO]` |
| `S4` | 收尾与完成确认 | `S3` | `[TODO]` |

#### 阶段 1：准备与基线确认

- 阶段目标：把报告中的仓库内优化项转成可执行回归基线，并确认当前失败模式
- 预计时间：1.5-2 小时
- 交付物：失败回归场景、基线测试记录、已知外部依赖说明
- 进入条件：计划已确认，开发环境可正常构建
- 完成条件：关键场景已被测试或明确标记为人工验证项

#### 阶段 2：核心实现

- 阶段目标：完成 HTTP resume 错误降级、连接生命周期收敛、日志增强、`connect_tcp` 校验
- 预计时间：5-7 小时
- 交付物：核心 Rust 代码改动
- 进入条件：阶段 1 基线已建立
- 完成条件：各核心任务对应测试逐项转绿，且共享连接语义未回退

#### 阶段 3：测试与验证

- 阶段目标：完成新增回归、全量测试、人工复现与日志核对
- 预计时间：2-3 小时
- 交付物：测试结果、人工验证记录
- 进入条件：阶段 2 已完成
- 完成条件：成功标准与验收映射中的自动/人工验证全部通过

#### 阶段 4：收尾与完成确认

- 阶段目标：更新文档、同步状态、确认无未处理 blocker
- 预计时间：1-2 小时
- 交付物：文档更新、最终状态记录
- 进入条件：阶段 3 已通过
- 完成条件：Definition of Done 全部满足

### 4.3 Task 列表（必须使用统一模板）

#### Task 1 [T1]: 建立问题回归基线

| 项目 | 内容 |
|------|------|
| 任务 ID | `T1` |
| 所属阶段 | `S1` |
| 依赖任务 | 无 |
| 目标 | 将报告中的服务端可控优化点转成明确的回归基线：resume 降级、连接收敛、日志关联、`connect_tcp` 误用提示 |
| 代码范围 | `src/server/integration_tests.rs`、必要时 `src/logging.rs` 对应测试区 |
| 预期改动 | 新增失败回归用例或测试占位，明确当前失败现状和目标行为 |
| 前置条件 | 仓库可构建，现有测试可运行 |
| 输出产物 | 基线测试、失败预期清单 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo build
cargo test test_http_multi_client_session_resume_stability -- --nocapture
cargo test test_http_closed_channel_resume_returns_recoverable_error -- --nocapture
cargo test test_http_stale_connection_is_pruned_from_registry -- --nocapture
cargo test test_connect_tcp_rejects_unix_socket_path_with_hint -- --nocapture
```

**通过判定**：

- [PASS] 目标场景均已落成测试入口或人工验证入口
- [PASS] 未完成能力会以“测试失败”暴露，而不是“无测试”
- [PASS] 既有多客户端共享测试仍可运行

**失败处理**：

- 失败后先确认测试夹具、端口、Neovim 实例与环境依赖是否正常
- 最多允许 2 次修复重试
- 超过阈值后升级为 `[BLOCKED]`，记录无法稳定复现的场景与原因

**门禁规则**：

- [BLOCK] 当前 Task 未验证通过，禁止进入下一个 Task

#### Task 2 [T2]: 实现 closed-channel resume 错误降级

| 项目 | 内容 |
|------|------|
| 任务 ID | `T2` |
| 所属阶段 | `S2` |
| 依赖任务 | `T1` |
| 目标 | 将 closed-channel resume 从 HTTP 500 改为可恢复错误，并提示重新初始化 session |
| 代码范围 | `src/main.rs`、必要时 `src/server/resources.rs` |
| 预期改动 | 在 HTTP service 边界分类 session/resume 错误，输出明确恢复语义 |
| 前置条件 | Task 1 中相关回归基线已建立 |
| 输出产物 | error normalizer 实现、回归测试通过 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo test test_http_closed_channel_resume_returns_recoverable_error -- --nocapture
cargo test test_http_stale_socket_does_not_break_shared_session -- --nocapture
```

**通过判定**：

- [PASS] closed-channel resume 不再返回 500
- [PASS] 响应明确提示当前 session 不可恢复，需要重新初始化
- [PASS] stale socket 测试未回退为 `Channel closed`

**失败处理**：

- 失败后先检查错误分类层级是否正确，避免在过低层做不可控改造
- 最多允许 2 次修复重试
- 超过阈值后升级为 `[BLOCKED]`，回到错误契约定义重新核对

**门禁规则**：

- [BLOCK] 当前 Task 未验证通过，禁止进入下一个 Task

#### Task 3 [T3]: 收敛共享连接池中的 stale connection

| 项目 | 内容 |
|------|------|
| 任务 ID | `T3` |
| 所属阶段 | `S2` |
| 依赖任务 | `T2` |
| 目标 | 避免失效连接继续被资源层或工具层视为有效连接，降低 `Not connected` / `Unknown` 噪音 |
| 代码范围 | `src/server/core.rs`、`src/server/tools.rs`、`src/neovim/{client,connection}.rs` |
| 预期改动 | 增加连接失效识别与注册表收敛逻辑，稳定 reconnect / disconnect / get_connection 行为 |
| 前置条件 | Task 2 已通过 |
| 输出产物 | 连接生命周期收敛实现、回归测试 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo test test_http_stale_connection_is_pruned_from_registry -- --nocapture
cargo test test_http_multi_client_shared_connection_visibility -- --nocapture
```

**通过判定**：

- [PASS] stale 连接不会继续以正常连接身份暴露
- [PASS] 多 session 共享连接可见性不被破坏
- [PASS] reconnect / disconnect 后无脏映射残留

**失败处理**：

- 失败后先区分“清理过度”还是“清理不足”
- 最多允许 2 次修复重试
- 超过阈值后升级为 `[BLOCKED]`，暂停扩大改动范围

**门禁规则**：

- [BLOCK] 当前 Task 未验证通过，禁止进入下一个 Task

#### Task 4 [T4]: 增强日志关联字段与 stream 健康观测

| 项目 | 内容 |
|------|------|
| 任务 ID | `T4` |
| 所属阶段 | `S2` |
| 依赖任务 | `T3` |
| 目标 | 在单份日志中串起 request、session、tool、connection、stream 生命周期 |
| 代码范围 | `src/logging.rs`、`src/main.rs`、`src/server/resources.rs`、必要时 `src/server/tools.rs` |
| 预期改动 | 统一上下文字段提取，记录客户端类型、连接时长、关闭原因、关键 ID |
| 前置条件 | Task 3 已通过 |
| 输出产物 | 日志增强实现、日志检查测试或人工验证记录 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo test test_debug_log_formatter_preserves_request_session_connection_context -- --nocapture
cargo run -- --http-port 8080 --http-host 127.0.0.1 --connect manual
# 人工：复现一次失败 exec_lua + resume 流程并检查 debug_log.txt
```

**通过判定**：

- [PASS] 单次故障链路可看到 `session_id`、`request_id`、`toolUseId`、`connection_id`
- [PASS] `/mcp` 连接建立和关闭日志包含持续时间与关闭原因
- [PASS] 日志格式统一且没有明显无界增长

**失败处理**：

- 失败后先收缩为公共 helper 或 HTTP 边界单点修复
- 最多允许 2 次修复重试
- 超过阈值后升级为 `[BLOCKED]`，记录缺失字段来源

**门禁规则**：

- [BLOCK] 当前 Task 未验证通过，禁止进入下一个 Task

#### Task 5 [T5]: 为 `connect_tcp` 增加 Unix socket 误用前置校验

| 项目 | 内容 |
|------|------|
| 任务 ID | `T5` |
| 所属阶段 | `S2` |
| 依赖任务 | `T4` |
| 目标 | 在工具入口直接识别“把 Unix socket 路径传给 `connect_tcp`”的误用，并给出明确纠错提示 |
| 代码范围 | `src/server/tools.rs`、必要时 `docs/tools.md` |
| 预期改动 | 对输入 target 做地址类型预判，对误用返回稳定错误消息 |
| 前置条件 | Task 4 已通过 |
| 输出产物 | 输入校验逻辑、回归测试 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo test test_connect_tcp_rejects_unix_socket_path_with_hint -- --nocapture
cargo test test_tcp_connection_lifecycle -- --nocapture
```

**通过判定**：

- [PASS] Unix socket 路径输入 `connect_tcp` 时立即返回清晰提示
- [PASS] 合法 TCP 地址连接能力不受影响
- [PASS] 错误消息明确包含“请使用 connect”或等价语义

**失败处理**：

- 失败后先确认规则没有误伤合法 TCP 地址
- 最多允许 2 次修复重试
- 超过阈值后升级为 `[BLOCKED]`，回到输入分类规则重新定义

**门禁规则**：

- [BLOCK] 当前 Task 未验证通过，禁止进入下一个 Task

#### Task 6 [T6]: 完成全量回归与人工复现验收

| 项目 | 内容 |
|------|------|
| 任务 ID | `T6` |
| 所属阶段 | `S3` |
| 依赖任务 | `T5` |
| 目标 | 证明本次优化不破坏既有共享连接语义，并覆盖所有新增目标 |
| 代码范围 | `src/server/integration_tests.rs`、测试命令、人工验收流程 |
| 预期改动 | 必要时补齐剩余断言，运行全量测试与人工复现 |
| 前置条件 | Task 5 已通过 |
| 输出产物 | 测试结果、人工验证记录 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
cargo build
./scripts/run-test.sh -- --show-output
pre-commit run --all-files
cargo run -- --http-port 8080 --http-host 127.0.0.1 --connect manual
# 人工：按报告失败顺序复现一次，确认不再出现 HTTP 500，且日志可直接定位
```

**通过判定**：

- [PASS] 全量测试通过
- [PASS] 手工复现中 closed-channel 场景不再表现为 HTTP 500
- [PASS] 多客户端与 stale socket 回归保持通过

**失败处理**：

- 失败后先区分为“新增回归失败”“旧回归失败”“人工复现失败”三类
- 最多允许 2 次修复重试
- 超过阈值后升级为 `[BLOCKED]`，停止进入收尾阶段

**门禁规则**：

- [BLOCK] 当前 Task 未验证通过，禁止进入下一个 Task

#### Task 7 [T7]: 更新文档并完成交付确认

| 项目 | 内容 |
|------|------|
| 任务 ID | `T7` |
| 所属阶段 | `S4` |
| 依赖任务 | `T6` |
| 目标 | 更新工具和使用文档，使恢复动作、错误契约和排障方式与实现一致 |
| 代码范围 | `docs/tools.md`、`docs/usage.md` |
| 预期改动 | 更新 `connect` / `connect_tcp` 边界、closed-channel 恢复说明、multi-client troubleshooting |
| 前置条件 | Task 6 已通过 |
| 输出产物 | 文档更新、最终状态同步 |
| 当前状态 | `[TODO]` |

**验证命令 / 检查方式**：

```bash
rg -n "connect_tcp|connect|Channel closed|session|resume|重新初始化" docs/tools.md docs/usage.md
pre-commit run --all-files
```

**通过判定**：

- [PASS] 文档明确说明 `connect_tcp` 不接受 Unix socket 路径
- [PASS] 文档明确说明 closed-channel / session closed 的恢复动作
- [PASS] 文档内容与最终实现行为一致

**失败处理**：

- 失败后先核对文档与实际响应语义是否一致
- 最多允许 2 次修复重试
- 超过阈值后升级为 `[BLOCKED]`，记录不一致点并等待人工确认

**门禁规则**：

- [BLOCK] 当前 Task 未验证通过，禁止将计划标记为完成

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

这份 Plan 不是静态文档，而是一个需要持续更新的状态机文档。

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

### 推荐状态同步格式

```text
阶段: S2 [DOING]
任务: T3 [DOING]
验证: [FAIL]
原因: <一句话说明失败点>
处理: <下一步动作>
```

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
- 客户端侧 Lua 生成策略未被误纳入本次实现范围
- 外部客户端不可控项已被明确记录为依赖/风险，而非伪装为“已完成”
- 文档、测试、实现三者保持一致

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
