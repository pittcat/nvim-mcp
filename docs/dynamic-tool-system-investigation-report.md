# Dynamic Tool System 调研排查报告

## 目标

排查 [docs/features-explained.md](/Users/pittcat/Dev/Rust/nvim-mcp/docs/features-explained.md#L7) 中 `Dynamic Tool System` 特性的真实状态、已知问题、实现边界与测试覆盖。

## 结论摘要

这个特性当前不是“不可用”，而是“happy path 可用、边界条件脆弱”。

- 正常路径已经打通：Lua `custom_tools` 可以被 Rust 侧发现、列出并调用。
- 该特性仍然符合文档里的“实验性”定位，而且还有几类未被测试覆盖的实现问题。
- 最大的问题不在主流程，而在严格 schema、异常注册、跨连接复用和字符串拼接安全性。

## 调研范围

- Lua 插件入口：[lua/nvim-mcp/init.lua](/Users/pittcat/Dev/Rust/nvim-mcp/lua/nvim-mcp/init.lua)
- 动态工具发现与执行：[src/server/lua_tools.rs](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/lua_tools.rs)
- 动态/静态混合路由：[src/server/hybrid_router.rs](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/hybrid_router.rs)
- 端到端测试：[src/server/integration_tests.rs](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/integration_tests.rs)
- 示例配置：[src/testdata/cfg_lsp.lua](/Users/pittcat/Dev/Rust/nvim-mcp/src/testdata/cfg_lsp.lua)

## 已验证现状

### 1. 主链路是通的

- Lua 侧通过 `M.setup()` 把 `custom_tools` 放进注册表：[lua/nvim-mcp/init.lua:95](/Users/pittcat/Dev/Rust/nvim-mcp/lua/nvim-mcp/init.lua#L95)
- Rust 侧连接后调用 `require('nvim-mcp').get_registered_tools()` 做发现：[src/server/lua_tools.rs:199](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/lua_tools.rs#L199)
- 工具注册进入 `HybridToolRouter`，按 `tool_name -> connection_id -> tool` 存储：[src/server/hybrid_router.rs:85](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/hybrid_router.rs#L85)
- 端到端测试覆盖了“发现 -> 列出 -> 调用 -> 断连后失效”这条主流程：[src/server/integration_tests.rs:1064](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/integration_tests.rs#L1064)

### 2. 当前测试结果

- `cargo build`：通过
- `cargo test --lib server::lua_tools::tests:: -- --nocapture`：通过，9 个单测
- `cargo test --lib hybrid_router::tests:: -- --nocapture`：通过，3 个单测
- `cargo test test_lua_tools_end_to_end_workflow -- --nocapture`：通过

注意：端到端测试依赖先生成 `target/debug/nvim-mcp`，否则会因测试基建直接 panic，而不是特性逻辑失败：[src/test_utils.rs:509](/Users/pittcat/Dev/Rust/nvim-mcp/src/test_utils.rs#L509)

## 主要问题

### P1: `connection_id` 自动注入和严格 JSON Schema 冲突

严重程度：高

文档声称动态工具会“自动获得 `connection_id` 参数”：[docs/features-explained.md:339](/Users/pittcat/Dev/Rust/nvim-mcp/docs/features-explained.md#L339)。实现上，这个注入只发生在对外展示的 MCP `Tool` schema 中：[src/server/hybrid_router.rs:27](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/hybrid_router.rs#L27)。

但真正执行时，`HybridToolRouter` 会把包含 `connection_id` 的完整参数送去校验：[src/server/hybrid_router.rs:305](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/hybrid_router.rs#L305)，而 `LuaToolConfig::validate_input()` 校验的仍是原始 `input_schema`：[src/server/lua_tools.rs:72](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/lua_tools.rs#L72)。

这意味着：

- 如果用户 schema 使用了 `additionalProperties = false`
- 或者 schema 明确禁止未声明字段

那么自动注入的 `connection_id` 会把本来合法的调用判成非法。

影响：

- 文档给人的心智模型是“用户不用管 `connection_id` schema”
- 实际上严格 schema 场景下会直接失效
- 当前测试没有覆盖这个边界

### P1: 动态工具名直接拼进 Lua 代码，未做转义

严重程度：高

调用动态工具时，Rust 直接把 `self.name` 拼进 Lua 字符串：

- [src/server/lua_tools.rs:102](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/lua_tools.rs#L102)

具体形式是：

```rust
"return require('nvim-mcp').execute_tool('{}', vim.json.decode({:?}))"
```

这里没有对工具名做 Lua 字符串级别转义。

影响：

- 工具名包含 `'`、反斜杠等字符时，生成的 Lua 代码会损坏
- 这是实际可触发的执行错误面
- 从安全角度看，也留下了代码注入面

文档和代码都没有限制 tool name 字符集。

### P1: 单个坏工具会拖垮该连接下整批动态工具注册，而且对调用方不透明

严重程度：高

在发现阶段，只要某个工具的 schema 初始化失败，就会直接返回错误，中断整个连接的动态工具发现：

- [src/server/lua_tools.rs:226](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/lua_tools.rs#L226)

注册阶段如果工具名与静态工具冲突，也会直接报错：

- [src/server/hybrid_router.rs:144](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/hybrid_router.rs#L144)

但上层 `setup_new_client()` 对这些错误只打 warning，不会把失败明确反馈给连接调用方：

- [src/server/core.rs:176](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/core.rs#L176)

影响：

- 一个坏工具会让该连接所有动态工具都不可用
- 用户看到的是“连接成功，但自定义工具没出现”
- 排障依赖服务端日志，不利于使用

### P2: 跨连接同名工具允许异构定义，但 `list_tools` 只展示其中一个版本

严重程度：中高

文档明确写了“不同 Neovim 实例可以有同名的不同工具实现”：[docs/features-explained.md:331](/Users/pittcat/Dev/Rust/nvim-mcp/docs/features-explained.md#L331)。

实现上，`list_all_tools()` 对同名动态工具只取“任意一个连接”的定义作为全局展示：

- [src/server/hybrid_router.rs:224](/Users/pittcat/Dev/Rust/nvim-mcp/src/server/hybrid_router.rs#L224)

代码里甚至写了注释：`they should all be the same`，但实际上并没有做任何一致性约束。

影响：

- 两个连接若注册了同名但不同 schema/description 的工具
- MCP `list_tools` 可能展示错误的参数结构
- 调用时只有到了具体 `connection_id` 才会暴露问题

这会让“全局工具列表”和“实际某连接可调用的工具定义”不一致。

### P2: `setup()` 是一次性的，后续动态变更不会刷新注册表

严重程度：中

Lua 入口里 `has_setup` 一旦置位，后续再次调用 `M.setup()` 会直接返回：

- [lua/nvim-mcp/init.lua:95](/Users/pittcat/Dev/Rust/nvim-mcp/lua/nvim-mcp/init.lua#L95)

影响：

- 二次调用 `setup()` 无法补充或更新 `custom_tools`
- 插件热重载、增量配置、运行中补注册都不会生效
- 目前也没有“重新发现动态工具”的显式刷新机制

如果产品预期只是“启动期静态配置”，这不算 bug；但如果文档给人的预期是“运行中可调整”，这里就是限制。

## 测试覆盖缺口

当前测试主要覆盖了 happy path，没有覆盖以下高风险场景：

- 严格 schema：`additionalProperties = false`
- 动态工具名含 `'`、`\\` 等特殊字符
- 同名工具在不同连接上 schema 不一致
- 单个坏工具存在时，其余工具是否还能部分注册
- 静态工具冲突时是否能向调用方提供明确错误
- 二次 `setup()` 或运行中变更 `custom_tools`

## 文档层面需要修正的点

`docs/features-explained.md` 的总体方向没错，但这几处表述偏乐观：

- “自动注入 `connection_id`”需要补充约束：严格 schema 当前不兼容
- “不同实例可以有同名不同实现”需要补充说明：全局 `list_tools` 元数据可能漂移
- “无限扩展”不准确，至少当前还受注册失败传播、命名安全、刷新机制限制

## 建议优先级

### 建议先修

1. 让执行时校验使用“注入 `connection_id` 后”的 schema，或在校验前剥离 `connection_id`
2. 对动态工具名做 Lua 安全转义，避免拼接执行
3. 将“单个工具失败”改成“跳过该工具并继续注册其他工具”
4. 把动态工具注册失败显式反馈给连接结果或资源面板

### 建议随后补上

1. 为跨连接同名工具增加一致性检查，或把 `list_tools` 改成 connection-aware 展示
2. 明确 `setup()` 的语义：只允许一次，还是支持更新/刷新
3. 增加对应单测和集成测试，覆盖上述边界条件

## 最终判断

这个特性当前可以继续保留，但不适合从“实验性”提升为“稳定特性”。

如果要继续对外强调它的可扩展性，至少应先解决：

- schema 注入与校验不一致
- tool name 字符串拼接风险
- 单点失败导致整批动态工具失效

否则用户一旦走出示例里的简单场景，就会遇到很难理解的失败模式。
