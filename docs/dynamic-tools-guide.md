# Dynamic Tools 开发指南

本指南详细介绍如何在 nvim-mcp 项目中添加 Dynamic Tools（动态工具），特别适用于管理 Window、Tab、Buffer 等 Neovim 对象的场景。

## 架构概述

Dynamic Tools 的工作流程：

```
┌─────────────────────────────────────────────────────────────────┐
│                         Rust MCP Server                         │
│  ┌─────────────────┐    ┌──────────────────────────────────┐   │
│  │  lua_tools.rs   │───▶│ discover_lua_tools()             │   │
│  │                 │    │   1. 调用 Lua 获取工具列表        │   │
│  │ LuaToolConfig  │    │   2. 解析工具配置                 │   │
│  │ DynamicTool    │    │   3. 注册为动态工具               │   │
│  └─────────────────┘    └──────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Neovim (Lua Plugin)                       │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ nvim-mcp/init.lua                                        │   │
│  │   - _tool_registry: 存储工具配置                          │   │
│  │   - setup(): 注册工具                                     │   │
│  │   - get_registered_tools(): 返回工具列表                  │   │
│  │   - execute_tool(): 执行工具                              │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## 添加新工具的两种方式

### 方式一：纯 Lua 实现（推荐）

最简单的方式是完全在 Lua 中实现工具，Neovim 端注册，Rust 端自动发现。

### 方式二：Rust + Lua 混合

工具定义在 Rust，但逻辑调用 Lua。

---

## 步骤一：定义工具处理器

在 Neovim 配置中，使用 `require('nvim-mcp').setup()` 注册工具。

### 基本结构

```lua
-- 在你的 Neovim 配置中 (init.lua 或 plugins.lua)
local nvim_mcp = require('nvim-mcp')

nvim_mcp.setup({
    custom_tools = {
        -- 工具名称
        ["tool_name"] = {
            -- 工具描述（必需）
            description = "工具的详细描述，会显示给 AI",

            -- JSON Schema 参数规范（必需）
            parameters = {
                type = "object",
                properties = {
                    param_name = {
                        type = "string",  -- string, number, boolean, array, object
                        description = "参数说明",
                    },
                },
                required = ["param_name"],  -- 必需参数列表
            },

            -- 工具处理函数（必需）
            handler = function(params)
                -- params 是从 MCP 传来的参数对象
                -- 返回格式必须符合 MCP 响应格式
            end,
        },
    },
})
```

### 响应格式

工具可以返回以下格式：

```lua
-- 成功响应（使用 MCP helper）
return nvim_mcp.MCP.success(data)
return nvim_mcp.MCP.text("纯文本消息")
return nvim_mcp.MCP.json({ key = "value" })

-- 错误响应
return nvim_mcp.MCP.error("ERROR_CODE", "错误消息")
```

---

## 步骤二：实现 Window 管理工具示例

以下是完整的 Window 管理工具实现：

```lua
-- 添加到你的 Neovim 配置中
local nvim_mcp = require('nvim-mcp')

nvim_mcp.setup({
    custom_tools = {
        -- 列出所有窗口
        ["window-list"] = {
            description = "列出当前标签页中的所有窗口",
            parameters = {
                type = "object",
                properties = {
                    tabnr = {
                        type = "number",
                        description = "标签页编号，默认为当前标签页",
                    },
                },
                required = {},
            },
            handler = function(params)
                local tabnr = params.tabnr or vim.api.nvim_get_current_tabpage()
                local wins = vim.api.nvim_tabpage_list_wins(tabnr)

                local window_list = {}
                for _, win in ipairs(wins) do
                    local bufnr = vim.api.nvim_win_get_buf(win)
                    local buf_name = vim.api.nvim_buf_get_name(bufnr)
                    local cursor = vim.api.nvim_win_get_cursor(win)

                    table.insert(window_list, {
                        id = win,
                        buffer = buf_name,
                        line = cursor[1],
                        column = cursor[2],
                        focused = vim.api.nvim_get_current_win() == win,
                    })
                end

                return nvim_mcp.MCP.success(window_list)
            end,
        },

        -- 创建新窗口
        ["window-create"] = {
            description = "创建新的窗口",
            parameters = {
                type = "object",
                properties = {
                    file = {
                        type = "string",
                        description = "要在新窗口中打开的文件路径",
                    },
                    vertical = {
                        type = "boolean",
                        description = "是否垂直分割，默认为 false",
                    },
                    split = {
                        type = "string",
                        description = "split 方式: 'left', 'right', 'top', 'bottom'",
                    },
                },
                required = {},
            },
            handler = function(params)
                local cmd = ""

                if params.vertical then
                    cmd = "vsplit"
                else
                    cmd = "split"
                end

                if params.split then
                    local directions = {
                        left = "topleft",
                        right = "botright",
                        top = "topleft vertical",
                        bottom = "botright",
                    }
                    cmd = directions[params.split] or cmd
                end

                if params.file and params.file ~= "" then
                    cmd = cmd .. " " .. vim.fn.fnameescape(params.file)
                end

                vim.cmd(cmd)

                return nvim_mcp.MCP.success({
                    message = "窗口已创建",
                    window = vim.api.nvim_get_current_win(),
                })
            end,
        },

        -- 关闭窗口
        ["window-close"] = {
            description = "关闭指定窗口",
            parameters = {
                type = "object",
                properties = {
                    winid = {
                        type = "number",
                        description = "窗口 ID，如果未指定则关闭当前窗口",
                    },
                    force = {
                        type = "boolean",
                        description = "是否强制关闭（忽略未保存的更改）",
                    },
                },
                required = {},
            },
            handler = function(params)
                local winid = params.winid or vim.api.nvim_get_current_win()

                if params.force then
                    vim.api.nvim_win_close(winid, true)
                else
                    -- 检查是否有未保存的更改
                    local bufnr = vim.api.nvim_win_get_buf(winid)
                    local modified = vim.api.nvim_buf_get_option(bufnr, "modified")

                    if modified then
                        return nvim_mcp.MCP.error(
                            "UNSAVED_CHANGES",
                            "窗口有未保存的更改，请先保存或使用 force=true"
                        )
                    end

                    vim.api.nvim_win_close(winid, false)
                end

                return nvim_mcp.MCP.success({ message = "窗口已关闭" })
            end,
        },

        -- 切换窗口焦点
        ["window-focus"] = {
            description = "将焦点移动到指定窗口",
            parameters = {
                type = "object",
                properties = {
                    direction = {
                        type = "string",
                        enum = { "left", "right", "up", "down", "next", "prev" },
                        description = "移动方向",
                    },
                    winid = {
                        type = "number",
                        description = "指定窗口 ID",
                    },
                },
                required = {},
            },
            handler = function(params)
                if params.winid then
                    -- 聚焦到指定窗口
                    vim.api.nvim_set_current_win(params.winid)
                elseif params.direction then
                    -- 按方向移动
                    local direction_map = {
                        left = "h",
                        right = "l",
                        up = "k",
                        down = "j",
                    }

                    local dir = direction_map[params.direction]
                    if not dir then
                        return nvim_mcp.MCP.error(
                            "INVALID_DIRECTION",
                            "无效方向: " .. params.direction
                        )
                    end

                    vim.cmd("wincmd " .. dir)
                else
                    return nvim_mcp.MCP.error(
                        "MISSING_PARAMS",
                        "必须提供 direction 或 winid 参数"
                    )
                end

                return nvim_mcp.MCP.success({
                    message = "已切换焦点",
                    window = vim.api.nvim_get_current_win(),
                })
            end,
        },
    },
})
```

---

## 步骤三：实现 Tab 管理工具示例

```lua
nvim_mcp.setup({
    custom_tools = {
        -- 列出所有标签页
        ["tab-list"] = {
            description = "列出所有标签页",
            parameters = {
                type = "object",
                properties = {},
                required = {},
            },
            handler = function(params)
                local tabs = vim.api.nvim_list_tabpages()
                local current_tab = vim.api.nvim_get_current_tabpage()

                local tab_list = {}
                for _, tab in ipairs(tabs) do
                    local wins = vim.api.nvim_tabpage_list_wins(tab)
                    local buf_count = 0

                    -- 统计窗口中的缓冲区
                    for _, win in ipairs(wins) do
                        buf_count = buf_count + 1
                    end

                    table.insert(tab_list, {
                        id = vim.api.nvim_tabpage_get_number(tab),
                        current = tab == current_tab,
                        window_count = buf_count,
                    })
                end

                return nvim_mcp.MCP.success(tab_list)
            end,
        },

        -- 创建新标签页
        ["tab-create"] = {
            description = "创建新的标签页",
            parameters = {
                type = "object",
                properties = {
                    file = {
                        type = "string",
                        description = "要在新标签页中打开的文件",
                    },
                },
                required = {},
            },
            handler = function(params)
                local cmd = "tabnew"
                if params.file and params.file ~= "" then
                    cmd = cmd .. " " .. vim.fn.fnameescape(params.file)
                end

                vim.cmd(cmd)

                return nvim_mcp.MCP.success({
                    message = "标签页已创建",
                    tabnr = vim.api.nvim_tabpage_get_number(vim.api.nvim_get_current_tabpage()),
                })
            end,
        },

        -- 关闭标签页
        ["tab-close"] = {
            description = "关闭指定标签页",
            parameters = {
                type = "object",
                properties = {
                    tabnr = {
                        type = "number",
                        description = "标签页编号，默认为当前标签页",
                    },
                    force = {
                        type = "boolean",
                        description = "是否强制关闭",
                    },
                },
                required = {},
            },
            handler = function(params)
                local tabnr = params.tabnr or vim.api.nvim_tabpage_get_number(
                    vim.api.nvim_get_current_tabpage()
                )
                local tabpage = vim.api.nvim_find_tabpage(tabnr)

                if not tabpage then
                    return nvim_mcp.MCP.error("INVALID_TAB", "标签页不存在")
                end

                -- 获取标签页中的所有窗口，检查是否有未保存的更改
                if not params.force then
                    local wins = vim.api.nvim_tabpage_list_wins(tabpage)
                    for _, win in ipairs(wins) do
                        local bufnr = vim.api.nvim_win_get_buf(win)
                        if vim.api.nvim_buf_get_option(bufnr, "modified") then
                            return nvim_mcp.MCP.error(
                                "UNSAVED_CHANGES",
                                "标签页中有未保存的更改"
                            )
                        end
                    end
                end

                vim.cmd(tabnr .. "tabclose")

                return nvim_mcp.MCP.success({ message = "标签页已关闭" })
            end,
        },

        -- 切换标签页
        ["tab-next"] = {
            description = "切换到下一个或指定标签页",
            parameters = {
                type = "object",
                properties = {
                    tabnr = {
                        type = "number",
                        description = "标签页编号",
                    },
                    relative = {
                        type = "string",
                        enum = { "next", "prev", "first", "last" },
                        description = "相对移动方式",
                    },
                },
                required = {},
            },
            handler = function(params)
                if params.tabnr then
                    vim.cmd(params.tabnr .. "tabnext")
                elseif params.relative then
                    local cmd_map = {
                        next = "tabnext",
                        prev = "tabprevious",
                        first = "tabfirst",
                        last = "tablast",
                    }
                    vim.cmd(cmd_map[params.relative])
                else
                    -- 默认切换到下一个
                    vim.cmd("tabnext")
                end

                return nvim_mcp.MCP.success({
                    message = "已切换到标签页",
                    tabnr = vim.api.nvim_tabpage_get_number(
                        vim.api.nvim_get_current_tabpage()
                    ),
                })
            end,
        },
    },
})
```

---

## 步骤四：实现 Buffer 管理工具（扩展现有功能）

```lua
nvim_mcp.setup({
    custom_tools = {
        -- 列出所有缓冲区
        ["buffer-list"] = {
            description = "列出所有缓冲区",
            parameters = {
                type = "object",
                properties = {
                    visible_only = {
                        type = "boolean",
                        description = "只显示可见的缓冲区",
                    },
                },
                required = {},
            },
            handler = function(params)
                local bufs = vim.api.nvim_list_bufs()
                local current_buf = vim.api.nvim_get_current_buf()

                local buffer_list = {}
                for _, bufnr in ipairs(bufs) do
                    local name = vim.api.nvim_buf_get_name(bufnr)
                    local loaded = vim.api.nvim_buf_is_loaded(bufnr)
                    local modified = vim.api.nvim_buf_get_option(bufnr, "modified")

                    -- 过滤条件
                    if params.visible_only and name == "" then
                        goto continue
                    end

                    table.insert(buffer_list, {
                        id = bufnr,
                        name = name,
                        loaded = loaded,
                        modified = modified,
                        current = bufnr == current_buf,
                    })

                    ::continue::
                end

                return nvim_mcp.MCP.success(buffer_list)
            end,
        },

        -- 删除缓冲区
        ["buffer-delete"] = {
            description = "删除指定的缓冲区",
            parameters = {
                type = "object",
                properties = {
                    bufnr = {
                        type = "number",
                        description = "缓冲区编号",
                    },
                    force = {
                        type = "boolean",
                        description = "强制删除",
                    },
                },
                required = {},
            },
            handler = function(params)
                local bufnr = params.bufnr or vim.api.nvim_get_current_buf()

                if not params.force then
                    local modified = vim.api.nvim_buf_get_option(bufnr, "modified")
                    if modified then
                        return nvim_mcp.MCP.error(
                            "UNSAVED_CHANGES",
                            "缓冲区有未保存的更改"
                        )
                    end
                end

                vim.api.nvim_buf_delete(bufnr, { force = params.force or false })

                return nvim_mcp.MCP.success({ message = "缓冲区已删除" })
            end,
        },
    },
})
```

---

## 高级功能

### 1. 使用枚举限制参数值

```lua
parameters = {
    type = "object",
    properties = {
        mode = {
            type = "string",
            enum = { "n", "v", "i", "t" },
            description = "Vim 模式: normal, visual, insert, terminal",
        },
    },
    required = ["mode"],
},
```

### 2. 添加数值范围限制

```lua
parameters = {
    type = "object",
    properties = {
        height = {
            type = "number",
            minimum = 1,
            maximum = 100,
            description = "窗口高度",
        },
    },
},
```

### 3. 使用数组类型参数

```lua
parameters = {
    type = "object",
    properties = {
        files = {
            type = "array",
            items = {
                type = "string",
            },
            description = "要打开的文件列表",
        },
    },
},
handler = function(params)
    for _, file in ipairs(params.files) do
        vim.cmd("edit " .. vim.fn.fnameescape(file))
    end
end,
```

### 4. 异步操作

如果工具需要执行异步操作，可以使用 Vim 的异步特性：

```lua
handler = function(params)
    vim.defer_fn(function()
        -- 延迟执行的代码
        vim.cmd("echo 'Done'")
    end, 1000)  -- 延迟 1000ms

    -- 立即返回
    return nvim_mcp.MCP.success({ message = "任务已启动" })
end,
```

---

## 完整配置示例

将以下代码添加到你的 Neovim 配置中：

```lua
-- init.lua 或 plugins.lua
local nvim_mcp = require('nvim-mcp')

nvim_mcp.setup({
    custom_tools = {
        -- Window 工具
        ["window-list"] = {
            description = "列出当前标签页中的所有窗口",
            parameters = {
                type = "object",
                properties = {
                    tabnr = {
                        type = "number",
                        description = "标签页编号",
                    },
                },
                required = {},
            },
            handler = function(params)
                -- 实现代码...
            end,
        },

        ["window-create"] = { /* ... */ },
        ["window-close"] = { /* ... */ },
        ["window-focus"] = { /* ... */ },

        -- Tab 工具
        ["tab-list"] = { /* ... */ },
        ["tab-create"] = { /* ... */ },
        ["tab-close"] = { /* ... */ },
        ["tab-next"] = { /* ... */ },

        -- Buffer 工具
        ["buffer-list"] = { /* ... */ },
        ["buffer-delete"] = { /* ... */ },
    },
})
```

---

## 调试技巧

### 1. 查看已注册的工具

在 Neovim 中运行：

```vim
:lua print(vim.inspect(require('nvim-mcp').get_registered_tools()))
```

### 2. 手动测试工具执行

```vim
:lua vim.print(require('nvim-mcp').execute_tool('window-list', {}))
```

### 3. 启用调试日志

在 Rust 端设置日志级别：

```bash
RUST_LOG=debug cargo run
```

---

## 常见问题

### Q: 工具没有被发现？

1. 确认 `nvim-mcp` 插件已正确加载
2. 检查 `setup()` 是否在 Neovim 启动时调用
3. 查看 Rust 服务日志中的警告信息

### Q: 工具执行失败？

1. 使用 `pcall` 包装 handler 代码查看错误
2. 检查返回格式是否符合 MCP 规范
3. 查看 Neovim 的消息日志

### Q: 参数验证失败？

1. 确保 JSON Schema 格式正确
2. 检查必需参数是否已提供
3. 验证参数类型是否匹配

---

## 执行插件命令与模拟按键

### 1. 执行已安装插件的命令

大多数 Neovim 插件会注册自己的 Ex 命令。你可以通过 `vim.cmd()` 或 `nvim_cmd()` 来执行这些命令：

```lua
nvim_mcp.setup({
    custom_tools = {
        -- 执行任意 Ex 命令
        ["execute-command"] = {
            description = "执行 Neovim Ex 命令",
            parameters = {
                type = "object",
                properties = {
                    command = {
                        type = "string",
                        description = "要执行的命令（如 :Git, :Goyo, :NvimTreeToggle 等）",
                    },
                },
                required = ["command"],
            },
            handler = function(params)
                -- 去除命令前的冒号（如果有）
                local cmd = params.command:gsub("^:", "")

                -- 安全地执行命令
                local success, err = pcall(vim.cmd, cmd)

                if success then
                    return nvim_mcp.MCP.success({
                        message = "命令已执行: " .. cmd,
                    })
                else
                    return nvim_mcp.MCP.error("COMMAND_FAILED", err)
                end
            end,
        },

        -- 执行 Telescope 搜索
        ["telescope-find-files"] = {
            description = "使用 Telescope 查找文件",
            parameters = {
                type = "object",
                properties = {
                    search_dir = {
                        type = "string",
                        description = "搜索目录，默认为当前目录",
                    },
                },
                required = {},
            },
            handler = function(params)
                local telescope = require("telescope.builtin")

                -- 调用 Telescope 的 find_files
                local ok, err = pcall(telescope.find_files, {
                    cwd = params.search_dir or vim.fn.getcwd(),
                })

                if ok then
                    return nvim_mcp.MCP.success({ message = "Telescope 已打开" })
                else
                    return nvim_mcp.MCP.error("TELESCOPE_ERROR", tostring(err))
                end
            end,
        },

        -- 执行 Lazy 包管理器命令
        ["lazy-sync"] = {
            description = "运行 Lazy.nvim 同步",
            parameters = {
                type = "object",
                properties = {},
                required = {},
            },
            handler = function(params)
                -- 调用 Lazy 的 sync 功能
                vim.cmd("Lazy! sync")

                return nvim_mcp.MCP.success({ message = "Lazy sync 已执行" })
            end,
        },
    },
})
```

### 2. 模拟按键输入

使用 `vim.api.nvim_feedkeys()` 来模拟按键：

```lua
nvim_mcp.setup({
    custom_tools = {
        -- 模拟按键
        ["send-keys"] = {
            description = "模拟按键输入",
            parameters = {
                type = "object",
                properties = {
                    keys = {
                        type = "string",
                        description = "要发送的按键（如 '<Esc>', '<CR>', 'i', 'gg 等）",
                    },
                    mode = {
                        type = "string",
                        enum = { "n", "i", "v", "c", "t" },
                        description = "按键模式: n=普通, i=插入, v=可视, c=命令行, t=终端",
                    },
                    escape = {
                        type = "boolean",
                        description = "是否需要转义特殊字符",
                    },
                },
                required = ["keys"],
            },
            handler = function(params)
                local keys = params.keys
                local mode = params.mode or "n"

                -- 处理特殊按键表示法
                if params.escape ~= false then
                    -- 将 \<Esc> 转换为真正的 Escape 字符
                    keys = vim.api.nvim_replace_termcodes(keys, true, true, true)
                end

                -- 添加到输入队列
                vim.api.nvim_feedkeys(keys, mode, false)

                return nvim_mcp.MCP.success({
                    message = "已发送按键: " .. params.keys,
                })
            end,
        },

        -- 常见快捷键预设
        ["keyboard-shortcut"] = {
            description = "执行常见快捷键操作",
            parameters = {
                type = "object",
                properties = {
                    action = {
                        type = "string",
                        enum = {
                            "escape",          -- 退出当前模式
                            "save",            -- 保存文件
                            "quit",            -- 退出
                            "quit_all",        -- 退出所有
                            "yank_all",       -- 全选复制
                            "delete_line",     -- 删除行
                            "goto_first",      -- 跳到开头
                            "goto_last",       -- 跳到结尾
                            "visual_line",     -- 进入行可视模式
                            "toggle_comment",  -- 切换注释
                        },
                        description = "要执行的操作",
                    },
                },
                required = ["action"],
            },
            handler = function(params)
                local keymap = {
                    escape = "<Esc>",
                    save = "<Esc>:w<CR>",
                    quit = "<Esc>:q<CR>",
                    quit_all = "<Esc>:qa<CR>",
                    yank_all = "ggVGy",
                    delete_line = "dd",
                    goto_first = "gg",
                    goto_last = "G",
                    visual_line = "V",
                    toggle_comment = "gcc",
                }

                local keys = keymap[params.action]
                if not keys then
                    return nvim_mcp.MCP.error("INVALID_ACTION", "未知操作")
                end

                -- 转换特殊键码
                keys = vim.api.nvim_replace_termcodes(keys, true, true, true)

                -- 在普通模式执行
                vim.api.nvim_feedkeys(keys, "n", false)

                return nvim_mcp.MCP.success({
                    message = "已执行快捷键: " .. params.action,
                })
            end,
        },

        -- 组合操作：进入插入模式并输入文本
        ["type-text"] = {
            description = "在当前光标位置输入文本（自动进入插入模式）",
            parameters = {
                type = "object",
                properties = {
                    text = {
                        type = "string",
                        description = "要输入的文本",
                    },
                    newline = {
                        type = "boolean",
                        description = "是否在文本后换行",
                    },
                },
                required = ["text"],
            },
            handler = function(params)
                -- 先进入插入模式
                vim.api.nvim_feedkeys("i", "n", false)

                -- 等待一小段时间确保模式切换
                vim.defer_fn(function()
                    -- 输入文本
                    vim.api.nvim_feedkeys(
                        vim.api.nvim_replace_termcodes(params.text, true, true, true),
                        "n",
                        false
                    )

                    -- 如果需要换行
                    if params.newline then
                        vim.api.nvim_feedkeys(
                            vim.api.nvim_replace_termcodes("<CR>", true, true, true),
                            "n",
                            false
                        )
                    end
                end, 10)

                return nvim_mcp.MCP.success({
                    message = "已输入文本",
                    text = params.text,
                })
            end,
        },
    },
})
```

### 3. 调用 Vim/Neovim 函数

许多插件会导出可通过 `vim.fn` 或 `vim.api.nvim_call_function` 调用的函数：

```lua
nvim_mcp.setup({
    custom_tools = {
        -- 调用任意 Vim 函数
        ["call-function"] = {
            description = "调用 Vim/Neovim 函数",
            parameters = {
                type = "object",
                properties = {
                    func = {
                        type = "string",
                        description = "函数名",
                    },
                    args = {
                        type = "array",
                        items = {
                            type = "any",
                        },
                        description = "函数参数",
                    },
                },
                required = ["func"],
            },
            handler = function(params)
                local func = params.func
                local args = params.args or {}

                -- 使用 pcall 安全调用
                local ok, result = pcall(vim.fn[func], unpack(args))

                if ok then
                    return nvim_mcp.MCP.success({
                        result = result,
                    })
                else
                    return nvim_mcp.MCP.error(
                        "FUNCTION_ERROR",
                        tostring(result)
                    )
                end
            end,
        },

        -- 示例：使用 fzf（如果安装了 fzf.vim）
        ["fzf-files"] = {
            description = "使用 fzf 模糊搜索文件",
            parameters = {
                type = "object",
                properties = {
                    prompt = {
                        type = "string",
                        description = "搜索提示符",
                    },
                },
                required = {},
            },
            handler = function(params)
                -- 调用 fzf 的Files 命令
                local ok = pcall(vim.cmd, "Files")

                if ok then
                    return nvim_mcp.MCP.success({ message = "FZF 已打开" })
                else
                    return nvim_mcp.MCP.error("FZF_NOT_FOUND", "FZF 未安装")
                end
            end,
        },
    },
})
```

---

## 相关文件

- `src/server/lua_tools.rs` - Rust 端工具发现与执行
- `lua/nvim-mcp/init.lua` - Neovim 端插件实现
- `docs/development.md` - 项目开发文档
