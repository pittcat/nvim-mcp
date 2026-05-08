# nvim-mcp 核心特性详解

本文档通俗易异地讲解 nvim-mcp 的两个核心特性：动态工具系统和插件集成。

---

## 1. Dynamic Tool System（动态工具系统）⚠️ 实验性

### 一句话概括

**让用户可以在 Neovim 的 Lua 配置中"自定义" MCP 工具，无需修改 Rust 代码。**

### 类比理解

想象 nvim-mcp 是一个餐厅：

| 类型 | 类比 | 说明 |
|------|------|------|
| **静态工具** | 菜单上的固定菜品 | Rust 代码里写死的工具，所有用户都一样 |
| **动态工具** | 顾客自己带的食材 | 用户在 Lua 里定义的工具，个性化定制 |

### 架构流程图

```
┌─────────────────────────────────────────────────────────────────┐
│  Neovim 侧 (Lua)                                                │
│  ───────────────────                                            │
│  用户在配置中注册自定义工具                                        │
│  ┌─────────────────────────────────────┐                        │
│  │  require('nvim-mcp').setup({        │                        │
│  │    custom_tools = {                 │                        │
│  │      my_tool = {                    │                        │
│  │        description = "我的工具",     │                        │
│  │        parameters = {...},          │                        │
│  │        handler = function(params)   │                        │
│  │          -- 工具逻辑                 │                        │
│  │        end                          │                        │
│  │      }                              │                        │
│  │    }                                │                        │
│  │  })                                 │                        │
│  └─────────────────────────────────────┘                        │
└───────────────────────┬─────────────────────────────────────────┘
                        │ 1. 注册到 M._tool_registry
                        │ 2. 启动 RPC server (serverstart)
                        ▼
┌─────────────────────────────────────────────────────────────────┐
│  MCP Server 侧 (Rust)                                           │
│  ───────────────────                                            │
│  连接时自动发现动态工具                                            │
│                                                                 │
│  ┌──────────────────┐    ┌──────────────────┐                  │
│  │  discover_lua_tools │ -> │ 检查插件是否可用  │                  │
│  │  (lua_tools.rs:199)│    │ check_plugin_... │                  │
│  └──────────────────┘    └──────────────────┘                  │
│           │                                                     │
│           ▼                                                     │
│  ┌─────────────────────────────────────────────────┐           │
│  │ 执行 Lua: require('nvim-mcp').get_registered_tools() │        │
│  │ 返回用户注册的所有工具定义                         │           │
│  └─────────────────────────────────────────────────┘           │
│           │                                                     │
│           ▼                                                     │
│  ┌─────────────────────────────────────────────────┐           │
│  │ 注册到 HybridToolRouter                         │           │
│  │ tool_name -> connection_id -> LuaToolConfig     │           │
│  │ 实现动态路由分发                                  │           │
│  └─────────────────────────────────────────────────┘           │
└─────────────────────────────────────────────────────────────────┘
```

### 关键代码文件

| 文件 | 作用 |
|------|------|
| `lua/nvim-mcp/init.lua` | Lua 插件入口，提供 `setup()` 和工具注册 API |
| `src/server/lua_tools.rs` | Rust 侧动态工具发现和执行逻辑 |
| `src/server/hybrid_router.rs` | 混合路由：静态工具 + 动态工具的统一分发 |

### 使用示例

```lua
-- 在用户的 init.lua 中
require('nvim-mcp').setup({
  custom_tools = {
    -- 自定义工具：获取当前文件的 Git 信息
    git_info = {
      description = "获取当前文件的 Git 提交历史",
      parameters = {
        type = "object",
        properties = {
          max_count = {
            type = "integer",
            description = "最大返回的提交数",
            minimum = 1,
            maximum = 50
          }
        },
        required = {"max_count"}
      },
      handler = function(params)
        local filename = vim.fn.expand('%')
        local cmd = string.format("git log -n %d %s",
          params.max_count, filename)

        local handle = io.popen(cmd)
        local result = handle:read("*a")
        handle:close()

        -- 使用 MCP 辅助函数返回结果
        return require('nvim-mcp').MCP.text(result)
      end
    }
  }
})
```

### MCP 辅助函数

在 Lua 中定义工具时，可以使用以下辅助函数构造返回值：

```lua
local MCP = require('nvim-mcp').MCP

-- 成功返回数据
MCP.success(data)        -- 返回 JSON 格式的成功响应
MCP.text("hello")        -- 返回纯文本响应
MCP.json({key = "value"}) -- 返回 JSON 响应

-- 返回错误
MCP.error("ERROR_CODE", "错误消息", optional_data)
```

---

## 2. Plugin Integration（插件集成）

### 一句话概括

**Neovim 插件自动设置，零配置即可使用。**

### 类比理解

就像买家电：

| 方式 | 类比 | 体验 |
|------|------|------|
| **传统方式** | 买回家自己接电线、接水管 | 手动配置 MCP server，麻烦 |
| **自动集成** | 送货上门，师傅帮你安装好 | 插件自动启动 RPC server，开箱即用 |

### 自动设置流程

```
┌─────────────────────────────────────────────────────────────┐
│  lazy.nvim 加载插件                                          │
│  ─────────────────                                          │
│  {                                                          │
│    "linw1995/nvim-mcp",                                     │
│    opts = { custom_tools = {...} }  ← 可选配置               │
│  }                                                          │
└─────────────────────┬───────────────────────────────────────┘
                      │ 插件加载
                      ▼
┌─────────────────────────────────────────────────────────────┐
│  lua/nvim-mcp/init.lua                                      │
│  ─────────────────────                                      │
│                                                             │
│  M.setup(opts)                                              │
│     │                                                       │
│     ├─► 1. 将 custom_tools 存入 M._tool_registry            │
│     │                                                       │
│     └─► 2. generate_pipe_path()                             │
│           生成 socket 路径: /tmp/nvim-mcp.{escaped_path}.{pid}.sock
│           基于: git 根目录 + 进程 ID                         │
│                                                               │
│     └─► 3. vim.fn.serverstart(pipe_path)                    │
│           启动 Neovim 内置 RPC server                        │
│           现在 MCP server 可以通过 socket 连接了！            │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### Socket 路径生成规则

```lua
-- 代码位置: lua/nvim-mcp/init.lua:79-91
local function generate_pipe_path()
  local git_root = get_git_root()  -- 获取 git 项目根目录
  local escaped_path = escape_path(git_root)  -- 替换 / 为 %
  local pid = vim.fn.getpid()      -- 获取 Neovim 进程 ID
  local temp_dir = "/tmp"          -- 临时目录

  -- 格式: /tmp/nvim-mcp.{escaped_git_root}.{pid}.sock
  return string.format("%s/nvim-mcp.%s.%d.sock",
    temp_dir, escaped_path, pid)
end
```

**示例路径：**
- 项目路径：`/home/user/my-project`
- 生成的 socket：`/tmp/nvim-mcp.%home%user%my-project.12345.sock`

### 自动连接机制

```rust
// 代码位置: src/server/core.rs

pub async fn connect_auto(&self) -> Result<(String, String), ServerError> {
    // 1. 获取当前工作目录
    let cwd = std::env::current_dir()?;

    // 2. 转义路径（替换 / 为 %）
    let escaped_project = escape_project_path(&cwd);

    // 3. 匹配 socket 模式: /tmp/nvim-mcp.{escaped_project}.*.sock
    let pattern = format!("/tmp/nvim-mcp.{}.*.sock", escaped_project);

    // 4. 自动发现并连接到匹配的 Neovim 实例
    for entry in glob::glob(&pattern)? {
        // ... 连接逻辑
    }
}
```

### 零配置完整流程

```
用户操作: 打开 Neovim 进入项目
    │
    ▼
┌──────────────────────────┐
│  lazy.nvim 自动加载插件   │
│  调用 require('nvim-mcp').setup() │
└──────────┬───────────────┘
           │
           ▼
┌──────────────────────────┐
│ 生成 socket 路径          │
│ /tmp/nvim-mcp.myproject.12345.sock │
└──────────┬───────────────┘
           │
           ▼
┌──────────────────────────┐
│ 启动 RPC server          │
│ vim.fn.serverstart()     │
└──────────┬───────────────┘
           │
           ▼
┌──────────────────────────┐
│ MCP Server 启动时         │
│ --connect auto           │
│ 自动发现 socket 并连接    │
└──────────────────────────┘
```

---

## 3. 两个特性的关系

### 整体架构图

```
┌─────────────────────────────────────────────────────────────┐
│                     nvim-mcp 架构                            │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────┐      ┌─────────────────────────────────┐  │
│  │   Neovim    │◄────►│          MCP Server             │  │
│  │             │ RPC  │                                 │  │
│  │  ┌───────┐  │ Socket│  ┌───────────┐  ┌───────────┐  │  │
│  │  │ Lua   │  │◄─────►│ │  Static   │  │  Dynamic  │  │  │
│  │  │ 插件  │  │      │ │  Tools    │  │  Tools    │  │  │
│  │  │ (setup)│  │      │ │  (Rust)   │  │  (Lua)    │  │  │
│  │  └───────┘  │      │ │           │  │           │  │  │
│  │      ▲      │      │ │ 内置工具   │  │ 用户自定义 │  │  │
│  │      │      │      │ │           │  │           │  │  │
│  │  ┌───┴───┐  │      │ └───────────┘  └───────────┘  │  │
│  │  │custom │  │      │         ▲            ▲        │  │
│  │  │tools  │──┘      │         └─────┬──────┘        │  │
│  │  └───────┘         │      HybridToolRouter         │  │
│  └─────────────────────┘            (统一路由)           │  │
│                              (hybrid_router.rs)            │
└─────────────────────────────────────────────────────────────┘
```

### 特性对比

| 特性 | 核心能力 | 用户价值 |
|------|----------|----------|
| **动态工具** | 用 Lua 写自定义工具 | 无需修改 Rust 代码，扩展无限可能 |
| **插件集成** | 自动启动 RPC server | 零配置，开箱即用 |

### 数据流总结

```
┌─────────────────────────────────────────────────────────┐
│  用户定义动态工具                                         │
│  (init.lua 中的 custom_tools)                            │
└────────────────────────┬────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│  Lua 插件注册到 M._tool_registry                         │
│  (lua/nvim-mcp/init.lua)                                 │
└────────────────────────┬────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│  MCP Server 发现工具                                     │
│  (discover_lua_tools 调用 get_registered_tools)         │
└────────────────────────┬────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│  注册到 HybridToolRouter                                 │
│  (动态工具与静态工具统一路由)                              │
└────────────────────────┬────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│  AI 助手通过 MCP 协议调用工具                              │
│  (根据 connection_id 路由到对应连接的工具实现)              │
└─────────────────────────────────────────────────────────┘
```

---

## 4. 关键设计亮点

### 4.1 连接隔离

每个动态工具都是 **per-connection** 的，不同 Neovim 实例可以有同名的不同工具实现。

```rust
// hybrid_router.rs 中的存储结构
pub type DynamicToolsStorage = Arc<DashMap<String, ConnectionToolMap>>;
//                      tool_name ──► connection_id ──► tool_impl
```

### 4.2 自动注入 connection_id

所有动态工具自动获得 `connection_id` 参数，用于路由到正确的 Neovim 连接。

```rust
// From<&dyn DynamicTool> for Tool 实现中
// 自动注入 connection_id 到 JSON Schema
properties
    .entry("connection_id".to_string())
    .or_insert_with(|| serde_json::json!({
        "type": "string",
        "description": "Unique identifier for the target Neovim instance"
    }));
```

### 4.3 参数验证

动态工具支持 JSON Schema 验证，在 Rust 侧使用 `jsonschema`  crate 进行校验。

```rust
// lua_tools.rs 中的 LuaToolValidator
pub fn validate(&self, params: &serde_json::Value) -> Result<(), String> {
    match jsonschema::is_valid(&self.schema, params) {
        true => Ok(()),
        false => Err("Validation failed: input does not match schema".to_string()),
    }
}
```

---

## 5. 使用场景举例

### 场景 1：项目特定的代码检查

```lua
custom_tools = {
  project_lint = {
    description = "运行项目特定的代码检查",
    handler = function(params)
      -- 调用项目内部的 lint 脚本
      local result = vim.fn.system("./scripts/custom-lint.sh")
      return MCP.text(result)
    end
  }
}
```

### 场景 2：集成内部工具

```lua
custom_tools = {
  query_internal_api = {
    description = "查询公司内部 API",
    parameters = {
      type = "object",
      properties = {
        endpoint = { type = "string" },
        params = { type = "object" }
      }
    },
    handler = function(params)
      -- 调用内部 CLI 工具
      local cmd = string.format("internal-cli %s", params.endpoint)
      -- ... 执行并返回
    end
  }
}
```

### 场景 3：自定义工作流

```lua
custom_tools = {
  create_pr_template = {
    description = "根据当前分支创建 PR 模板",
    handler = function(params)
      local branch = vim.fn.system("git branch --show-current")
      local template = generate_pr_template(branch)  -- 自定义逻辑
      return MCP.text(template)
    end
  }
}
```

---

## 总结

这两个特性结合起来，让 nvim-mcp 成为一个**可扩展的桥梁**：

1. **插件集成** 提供了**零配置**的体验，让用户无需关心底层的 RPC 连接
2. **动态工具** 提供了**无限扩展**的能力，让用户可以根据自己的需求定制工具

静态工具提供了开箱即用的基础能力，动态工具则让每个用户和项目都可以拥有专属的工具集。