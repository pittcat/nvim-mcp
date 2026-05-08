# Neovim 配置与使用说明

这份文档专门说明 `nvim-mcp` 在 **Neovim 侧** 应该怎么配置，以及配置完成后如何让 Claude Code 等 MCP 客户端接入。

## 1. 这个插件到底做了什么

Neovim 里的 `nvim-mcp` 插件主要负责两件事：

1. 在当前 Neovim 实例里启动一个 RPC socket。
2. 可选地注册你自定义的 Lua 工具，供 MCP server 动态发现。

也就是说：

- `lua/nvim-mcp/init.lua` 负责在 Neovim 里执行 `setup()`
- `nvim-mcp` 二进制负责作为 MCP server 连接这些 Neovim 实例

最常见的组合是：

1. 打开 Neovim
2. 插件自动启动 socket
3. MCP 客户端通过 `nvim-mcp --connect auto` 自动连接当前项目对应的 Neovim

## 2. 最小可用配置

推荐用 `lazy.nvim`：

```lua
return {
    {
        "linw1995/nvim-mcp",
        opts = {},
    },
}
```

这已经够用。`opts = {}` 会触发 `require("nvim-mcp").setup()`，插件会自动为当前项目生成 socket。

## 3. 自动安装服务端二进制

如果你希望插件安装时顺便安装 `nvim-mcp` 命令，可以加 `build`。

### Cargo 方式

```lua
return {
    {
        "linw1995/nvim-mcp",
        build = "cargo install --path .",
        opts = {},
    },
}
```

### Nix 方式

```lua
return {
    {
        "linw1995/nvim-mcp",
        build = [[
          nix build .#nvim-mcp
          nix profile remove nvim-mcp
          nix profile install .#nvim-mcp
        ]],
        opts = {},
    },
}
```

如果你不是直接在仓库里做本地开发，更常见的是手动安装：

```bash
cargo install nvim-mcp
```

## 4. 本地仓库开发模式

如果你正在本地开发这个仓库本身，例如仓库就在：

```text
/Users/pittcat/Dev/Rust/nvim-mcp
```

那么推荐把 Neovim 插件直接指向本地目录，而不是 GitHub 仓库地址。

### `lazy.nvim` 本地目录配置

```lua
return {
    {
        dir = "/Users/pittcat/Dev/Rust/nvim-mcp",
        name = "nvim-mcp",
        opts = {},
    },
}
```

这表示：

- Neovim 直接加载你本地仓库里的 `lua/nvim-mcp/init.lua`
- `opts = {}` 会执行 `require("nvim-mcp").setup({})`
- 每次改动本地 Lua 插件代码后，重启 Neovim 就能验证

### 本地目录 + 自动编译二进制

如果你希望 `lazy.nvim` 在插件更新或重载时顺便编译服务端二进制，可以这样写：

```lua
return {
    {
        dir = "/Users/pittcat/Dev/Rust/nvim-mcp",
        name = "nvim-mcp",
        build = "cargo build --release",
        opts = {},
    },
}
```

### 本地开发时的推荐职责划分

本地开发时建议这样理解：

1. **Neovim 配置**
   - 负责加载本地插件目录
   - 负责执行 `setup()`
   - 负责让当前 Neovim 实例创建 socket

2. **Claude Code 的 `.mcp.json`**
   - 负责启动你本地 `target/release/nvim-mcp` 二进制
   - 负责通过 `--connect auto` 自动连接 Neovim

也就是说，Neovim 配的是“插件目录”，Claude Code 配的是“服务端二进制路径”，两者不要混在一起。

### 本地开发对应的 `.mcp.json` 示例

如果你希望 Claude Code 使用你本地编译出来的二进制，可以这样写：

```json
{
  "mcpServers": {
    "nvim": {
      "command": "/Users/pittcat/Dev/Rust/nvim-mcp/target/release/nvim-mcp",
      "args": [
        "--connect",
        "auto",
        "--log-file",
        "./nvim-mcp.log",
        "--log-level",
        "debug"
      ],
      "env": {}
    }
  }
}
```

使用前先编译：

```bash
cargo build --release
```

### 本地开发模式下的完整流程

1. 在本地仓库里执行：

```bash
cargo build --release
```

2. Neovim 用本地插件目录配置加载 `nvim-mcp`
3. 打开当前项目目录下的 Neovim
4. Claude Code 在项目根目录读取 `.mcp.json`
5. Claude Code 启动本地 `target/release/nvim-mcp`
6. `nvim-mcp --connect auto` 自动发现并连接当前项目对应的 Neovim

## 4. Claude Code / MCP 客户端怎么接

推荐自动连接当前项目：

```bash
claude mcp add -s local nvim -- nvim-mcp --log-file . \
  --log-level debug --connect auto
```

含义是：

- `--connect auto`：自动寻找当前项目对应的 Neovim socket
- `--log-file .`：把日志写到当前目录
- `--log-level debug`：方便排查连接问题

如果你已经打开了该项目目录下的 Neovim，客户端通常会自动发现并连接上。

## 5. `custom_tools` 自定义工具

你可以把 Lua 函数暴露成 MCP 动态工具：

```lua
return {
    {
        "linw1995/nvim-mcp",
        opts = {
            custom_tools = {
                current_file_path = {
                    description = "Get current buffer absolute path",
                    parameters = {
                        type = "object",
                        properties = {},
                    },
                    handler = function(_)
                        local path = vim.api.nvim_buf_get_name(0)
                        return require("nvim-mcp").MCP.json({
                            path = path,
                        })
                    end,
                },
            },
        },
    },
}
```

规则如下：

- `description` 必填
- `handler` 必填
- `parameters` 建议按 JSON Schema 写
- 返回值建议使用：
  - `require("nvim-mcp").MCP.text(...)`
  - `require("nvim-mcp").MCP.json(...)`
  - `require("nvim-mcp").MCP.success(...)`
  - `require("nvim-mcp").MCP.error(...)`

## 6. 手动连接模式

如果你不想用插件自动生成 socket，也可以手动让 Neovim 监听：

```bash
nvim --listen 127.0.0.1:6666
```

或者：

```bash
nvim --listen ./nvim.sock
```

也可以直接写进配置：

```lua
vim.fn.serverstart("127.0.0.1:6666")
-- 或
vim.fn.serverstart("./nvim.sock")
```

然后启动服务端：

```bash
nvim-mcp --connect 127.0.0.1:6666
```

或者：

```bash
nvim-mcp --connect /绝对路径/nvim.sock
```

## 7. 插件是如何识别“当前项目”的

插件会优先取 Git 根目录；如果当前目录不在 Git 仓库中，则回退到 `vim.fn.getcwd()`。

生成的 socket 形式大致是：

```text
/tmp/nvim-mcp.{项目路径转义后}.{pid}.sock
```

因此：

- 在项目根目录打开 Neovim，`--connect auto` 效果最好
- 同一个项目开多个 Neovim 也可以，被自动发现

## 8. 排错建议

### 看不到连接

先确认：

1. Neovim 已经打开
2. 插件已加载并执行 `setup()`
3. `nvim-mcp` 命令可执行
4. MCP 客户端启动参数用了 `--connect auto`

### 确认插件是否加载

在 Neovim 里执行：

```vim
:lua print(vim.inspect(require("nvim-mcp")))
```

### 查看日志

```bash
nvim-mcp --log-file ./nvim-mcp.log --log-level debug --connect auto
```

### 检查 socket

在 macOS/Linux 上可以看 `/tmp`：

```bash
ls /tmp | grep nvim-mcp
```

## 9. 推荐使用方式

如果你只是想稳定可用，直接用下面这套：

### Neovim

```lua
return {
    {
        "linw1995/nvim-mcp",
        opts = {},
    },
}
```

### 安装服务端

```bash
cargo install nvim-mcp
```

### 配置 MCP 客户端

```bash
claude mcp add -s local nvim -- nvim-mcp --connect auto
```

这是最省心的默认方案。

  claude mcp add -s user nvim -- nvim-mcp --connect auto
