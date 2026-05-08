# 当前目标
当前围绕 `nvim-mcp` 的内存占用与连接形态做排查、测量和文档化，用户关心的是“真实使用场景下 `nvim-mcp` 占多少内存、为什么会有很多连接/进程、怎么优化”。预期产出包括：

- 一份中文分析报告，说明 `stdio` 多进程、共享 daemon、连接表和实际内存测量结果。
- 一个可执行的 Python 脚本，能启动 `target/release/nvim-mcp`，用最小 MCP 客户端真实握手后测 RSS，并把结果保存到 JSON。
- 对当前机器现场状态的排查结论：当前有哪些 `nvim-mcp` 进程、有哪些连接、哪些是测试残留、哪些与真实会话无关。

验收标准：

- 报告在 `docs/mcp-memory-optimization-report-zh.md`，且包含用户关心的真实测量口径。
- 脚本 `scripts/measure_connected_mcp_memory.py` 能运行，并生成 `target/benchmarks/memory/*.json`。
- 下一位 Agent 能根据本交接文档直接继续处理“是否清理残留 headless nvim”“是否继续收敛报告内容”“是否做后续优化改造”。

完成标准：

- 当前这一轮没有要求继续编码新功能。
- 最新现场里，`nvim-mcp` 进程已被 kill 掉，但 9 个 headless 测试 `nvim` 仍存活，尚未清理。

# 已完成内容
- 完成了内存优化分析报告初稿并多次修订，文件为 `docs/mcp-memory-optimization-report-zh.md`。
- 实现了最小真实 stdio 客户端握手后的内存测量脚本 `scripts/measure_connected_mcp_memory.py`。
- 删除了用户认为“无关紧要”的新增 Python 脚本，只保留 `measure_connected_mcp_memory.py`。
- 实测确认 `nvim-mcp` 的 `stdio` 通讯不是 `Content-Length` framing，而是“每条消息一行 JSON”。
- 实测生成了多份 JSON 结果，保存在 `target/benchmarks/memory/`。
- 多次排查当前机器现场状态，确认：
  - 共享 `nvim-mcp` 进程曾维护过 9 个连接。
  - 这 9 个连接都指向 `nvim-mcp` 仓库下的 headless 测试 `nvim`，不是 `mermaid_validator_rs` 的交互会话。
  - 已按用户要求 kill 掉当时存活的 `nvim-mcp` 进程。
  - kill 后，9 个 headless 测试 `nvim` 依然存活。

# 尝试过什么（包括成功和失败/死路）
## 报告与测量口径
- 先做了 debug/release 混合 benchmark。
  - 成功拿到一批数据，但不符合用户真正要的口径。
  - 用户明确指出他更关心“真实 `release` 使用态”和“真实客户端连接后的实际内存”。
- 后续把重点切到 `release` 和真实握手测量。
  - 这是正确方向。
  - 用户接受这个方向。

## 内存测量脚本
- 先做了 `bench_memory.py` 一类 synthetic benchmark。
  - 技术上可运行，但用户不需要这种模式。
  - 已删除。
- 改为 `scripts/measure_connected_mcp_memory.py`。
  - 成功。
  - 该脚本会启动 `target/release/nvim-mcp`，起一个或多个最小 Python MCP 客户端，完成真实握手，然后采集 `nvim-mcp` RSS。
  - 但当前是裸启动 `nvim-mcp`，没有附带用户日常配置里的 `--connect auto --log-file ./nvim-mcp.log --log-level debug`。
- 曾误以为 `stdio` 需要 `Content-Length` framing。
  - 失败现象：服务端报 `serde error expected value`。
  - 结论：这是错误方向。
- 改为 newline-delimited JSON。
  - 成功。
  - 已实际完成 `initialize`、`notifications/initialized`、`tools/list` 三步。

## 脚本清理
- 新增过如下脚本：
  - `scripts/bench_memory.py`
  - `scripts/test_bench_memory.py`
  - `scripts/test_measure_connected_mcp_memory.py`
- 用户明确要求删除“无关紧要的脚本”，最终只保留 `scripts/measure_connected_mcp_memory.py`。
  - 已执行删除。

## 现场进程与连接排查
- 一开始按“工作目录下的 `target/release/nvim-mcp`”去筛进程。
  - 在 `mermaid_validator_rs` 场景下这是误判来源。
  - 因为真实 MCP 进程是从 `/Users/pittcat/Dev/Rust/nvim-mcp/target/release/nvim-mcp` 共享出来的，不在 `mermaid_validator_rs` 仓库内。
- 后续改为：
  - 查共享 `nvim-mcp` 进程路径
  - 查 `nvim-mcp.log`
  - 查 `/tmp` socket
  - 查 socket 所属 PID
  - 查 PID 的 `ps` 命令行
  - 这条路线成功解释了“为什么 Claude Code 里看得到连接，而按项目路径查不到对应 `nvim-mcp` 进程”。

## 当前连接表解析
- 从 `nvim-mcp.log` 里提取 `get_targets` / `Active Connections` 信息。
  - 成功找到 9 个 `connection_id -> socket`。
- 试图从日志里用通用正则提取“当前块”时，日志中 MCP tool schema 和历史输出混在一起，容易误截。
  - 这是半死路。
  - 最后通过定位特定时间段和结合 `lsof`/`ps` 结果，仍然拿到了可靠结论。

## kill 进程
- 用户要求 kill 掉这些 `nvim-mcp`。
  - 已成功 kill 掉当时运行的 `target/release/nvim-mcp` 进程。
  - kill 之后复查，没有残留 `target/release/nvim-mcp` 进程。
- 用户随后要求确认那 9 个 headless `nvim` 是否还在。
  - 已确认全部仍存活。

# 关键决策和理由
- 决定只保留 `release` 口径作为主结论。
  - 原因：用户明确不需要 debug 数据，且 debug 数据与真实使用场景偏差较大。
  - 放弃方案：继续用 debug benchmark 作为主要结论。
- 决定把测量脚本收敛为“最小真实 stdio 客户端握手后测 RSS”。
  - 原因：它先回答了最核心的单实例/多实例基线问题。
  - 放弃方案：继续维护 synthetic benchmark/live-snapshot 多模式脚本体系。
- 决定保留 `scripts/measure_connected_mcp_memory.py`，删除其他新增 Python 脚本。
  - 原因：用户明确要求精简。
- 决定把“9 个连接”的解释落实为“1 个共享 `nvim-mcp` 进程内部维护 9 条 Neovim 连接”。
  - 原因：这是事实，也能解释用户对“为什么连接这么多”的疑问。
- 决定把 `mermaid_validator_rs` 场景下的“Target”解释为 Neovim socket，不是 `nvim-mcp` 进程路径。
  - 原因：这是此前误解的根因，必须明确。

# 修改过的文件及改动说明
- `docs/mcp-memory-optimization-report-zh.md`
  - 修改内容：
    - 多次重写和修订内存优化报告。
    - 加入 `release` 口径说明。
    - 加入 `scripts/measure_connected_mcp_memory.py` 的使用方式。
    - 修正过一条错误命令，正确命令为：
      `python3 scripts/measure_connected_mcp_memory.py --binary ./target/release/nvim-mcp --skip-build --client-count 3`
  - 为什么改：
    - 用户要求一份可执行、可落地、贴近真实场景的报告。
  - 是否已验证：
    - 文档内容与脚本和现场排查大体对齐。
    - 未逐段重新全文审校，仍可能残留历史表述。
  - 后续联动：
    - 建议下一位 Agent 再做一次全文一致性检查，确认是否仍残留已删除脚本或过期 benchmark 描述。

- `scripts/measure_connected_mcp_memory.py`
  - 修改内容：
    - 实现最小 MCP 客户端，通过 stdio 与 `target/release/nvim-mcp` 完成真实握手。
    - 支持 `--client-count`。
    - 测量 `nvim-mcp` 的 RSS。
    - 把结果写入 `target/benchmarks/memory/`。
    - 文件名带微秒时间戳，避免结果覆盖。
  - 为什么改：
    - 用户要比 synthetic benchmark 更贴近实际的测量口径。
  - 是否已验证：
    - 已实际运行成功。
  - 后续联动：
    - 当前没有配套测试文件，因为用户要求删除多余脚本。
    - 当前脚本仍未覆盖用户常用的 `--connect auto` 启动参数；如果后续要测“完整真实使用态”，需要先补这个能力或至少在文档中持续标注范围。

- `docs/plans/2026-03-15-memory-benchmark-design.md`
  - 修改内容：
    - 记录过早期 benchmark 设计方案。
  - 为什么改：
    - 为前期测量思路留档。
  - 是否已验证：
    - 文档本身未重新清理。
  - 后续联动：
    - 可能仍引用已删除脚本或过时方案，建议如继续保留则需要清理。

- `findings.md`
  - 修改内容：
    - 记录过阶段性发现。
  - 是否已验证：
    - 未重新审校。
  - 后续联动：
    - 可能包含历史中间态描述。

- `progress.md`
  - 修改内容：
    - 记录过阶段性进展。
  - 是否已验证：
    - 未重新审校。
  - 后续联动：
    - 同上，可能包含历史中间态。

- `docs/handoff/260315-handoff.md`
  - 修改内容：
    - 本交接文档。
  - 为什么改：
    - 用户明确要求生成面向下一位 Agent 的交接文档。
  - 是否已验证：
    - 已写入仓库。
  - 后续联动：
    - 无。

# 障碍 / 待决问题
- `docs/mcp-memory-optimization-report-zh.md` 是否仍残留过期 benchmark 口径或已删除脚本引用，未确认。
- `docs/plans/2026-03-15-memory-benchmark-design.md`、`findings.md`、`progress.md` 可能含历史中间态，未清理。
- 当前 9 个 headless 测试 `nvim` 仍在，占用总内存约 `45 MiB` 左右，是否按用户下一步意图清理，尚未执行。
- `mermaid_validator_rs` 的“当前真实可用连接”在最后一次现场检查时未再次确认，只能确认此前曾通过共享 `nvim-mcp` 连接成功。
- 日志解析不是强结构化接口，若继续依赖 `nvim-mcp.log` 做自动判断，容易被 tool schema 文本污染。

# 下一步计划（具体步骤）
1. 先确认用户下一步是否要清理 9 个 headless 测试 `nvim`。
   - 用户已经明确表示“1”，即先确认它们是否还活着，这一步已完成。
   - 如果用户继续要求清理，可直接 kill 这 9 个 PID：
     `97760 88410 97497 84537 88321 84535 97672 84276 88147`
2. 全文审查 `docs/mcp-memory-optimization-report-zh.md`。
   - 搜索 `bench_memory`、`live-snapshot`、`debug`、已删除脚本名。
   - 删除与当前最终方案不一致的历史内容。
3. 审查 `docs/plans/2026-03-15-memory-benchmark-design.md`、`findings.md`、`progress.md`。
   - 视需要标记“历史记录，仅供参考”或清理过期内容。
4. 如果用户还要继续优化产品本身，而不只是写报告：
   - 从 `stdio` 多进程重复启动问题入手，设计“同目录一个共享 daemon”的落地方案。
   - 这一步还没开始编码，不要误以为已经做了实现。

## 1. 当前任务目标
从接手视角看，当前最直接的目标是接住这条工作流的最后状态：

- 确认和处理当前机器上残留的 headless 测试 `nvim`
- 维护并收敛内存分析报告，使其只保留用户真正关心的 `release + 真实客户端连接后` 口径
- 在需要时，基于现有测量结果继续推进“共享 daemon 减少多 `stdio` 进程”的后续设计或实现

完成标准：

- 现场状态清晰，残留进程是否保留有明确决定
- 报告不再含主要误导性历史口径
- 若继续开发，有明确的下一步方案

## 2. 当前进展
- 报告已经存在并多次更新。
- 真实测量脚本已经存在并能工作。
- 多余脚本已经删掉。
- 共享 `nvim-mcp` 的连接表和当前 headless 测试 `nvim` 的来源已经查清。
- 共享 `nvim-mcp` 进程已被 kill。
- 9 个测试用 headless `nvim` 仍未清理。

## 3. 关键上下文
- 用户明确要求：
  - 报告用中文。
  - 需要 Markdown、Mermaid、ASCII。
  - 关注真实内存占用和多会话/多进程下的优化空间。
  - 在提出建议或疑问后，如要继续编码，先向用户确认。
- 用户最终只接受保留一个测量脚本：
  - `scripts/measure_connected_mcp_memory.py`
- 用户不需要 debug benchmark 作为主结论。
- 用户对“真实场景”定义为：
  - 启动 `target/release/nvim-mcp`
  - 让真实客户端连上
  - 再测 RSS
- 但按当前复核，用户日常配置还包含：
  - `--connect auto`
  - `--log-file ./nvim-mcp.log`
  - `--log-level debug`
- 因此现有脚本只覆盖了“真实 stdio 客户端握手”这层，不覆盖“自动连接 Neovim 后的完整运行态”。
- 当前仓库未提交状态较脏，`git status --short` 中有较多未跟踪文件。
- 当前对连接信息的很多判断来自 `nvim-mcp.log`、`ps`、`lsof`、`/tmp` socket，而不是一个稳定 API。

## 4. 关键发现
- `nvim-mcp` 的 stdio framing 是 newline-delimited JSON，不是 `Content-Length`。
- 单看真实客户端握手后的 `release nvim-mcp`，量级大约在 `7-8 MiB` 每实例，具体取决于现场条件。
- 最新 3-client 基线结果文件为 `target/benchmarks/memory/mcp-client-rss-20260315T101712010077Z.json`：
  - 合计 `22.69 MiB`
  - 分进程 `7.47 MiB`、`7.61 MiB`、`7.61 MiB`
- “现场快照里小于 4-5 MiB” 和 “真实客户端连上后 7-8 MiB” 并不矛盾，前者通常是更 idle 的 steady-state 快照。
- 曾看到的“9 个活动连接”不是 9 个 `nvim-mcp` 进程，而是 1 个共享 `nvim-mcp` 进程内部维护的 9 条 Neovim 连接。
- 这 9 条连接全部是 `nvim-mcp` 仓库测试场景产生的 headless `nvim`，不是 `mermaid_validator_rs` 的交互编辑会话。
- kill `nvim-mcp` 不会自动 kill 那 9 个 headless `nvim`。

## 5. 未完成事项
1. 是否 kill 这 9 个 headless 测试 `nvim`。
2. 清理报告中的历史口径和潜在过期描述。
3. 决定是否继续做产品级优化实现，例如共享 daemon、连接回收、lazy tools。
4. 清理或标注 `docs/plans/2026-03-15-memory-benchmark-design.md`、`findings.md`、`progress.md` 的历史内容。

## 6. 建议接手路径
- 先看 `scripts/measure_connected_mcp_memory.py`，理解“真实客户端握手测量”的当前实现。
- 再看 `docs/mcp-memory-optimization-report-zh.md`，检查是否还残留不一致内容。
- 若要处理现场残留，先用 `ps` 复核 9 个 headless `nvim` PID 是否仍存活，再决定是否 kill。
- 若要继续做架构优化，再去看：
  - `src/main.rs`
  - `src/server/core.rs`
  - `src/server/hybrid_router.rs`
  - `src/neovim/client.rs`
  - `src/neovim/connection.rs`

## 7. 风险与注意事项
- 不要再把 `mermaid_validator_rs` 的 Neovim socket 当成 `nvim-mcp` 进程路径。
- 不要把 “9 个连接” 理解成 “9 个 MCP 进程”。
- 不要再拿 debug benchmark 当主结论。
- `nvim-mcp.log` 的解析容易被 schema 文本污染，依赖它时要结合 `ps`/`lsof` 交叉验证。
- 用户对“先确认再编码”比较敏感，后续若要继续实现新功能，先确认范围。

# 重要注意事项（gotchas、约束、环境变量等）
- 当前工作目录：`/Users/pittcat/Dev/Rust/nvim-mcp`
- 当前日期：`2026-03-15`
- 用户语言偏好：中文
- 用户明确要求：
  - 报告是给他看的，交接文档是给下一位 Agent 看的
  - 继续编码前先确认
- 真实测量脚本命令：
  - `python3 scripts/measure_connected_mcp_memory.py --binary ./target/release/nvim-mcp --skip-build`
  - `python3 scripts/measure_connected_mcp_memory.py --binary ./target/release/nvim-mcp --skip-build --client-count 3`
- 用户曾因为多打空格而执行失败：
  - 错误形式：`python3 scripts/  measure_connected_mcp_memory.py ...`
  - 正确形式：`python3 scripts/measure_connected_mcp_memory.py ...`
- 当前已确认：
  - 没有运行中的 `target/release/nvim-mcp`
  - 有 9 个 headless 测试 `nvim` 仍在
- 当前 9 个 headless `nvim` PID：
  - `97760`
  - `88410`
  - `97497`
  - `84537`
  - `88321`
  - `84535`
  - `97672`
  - `84276`
  - `88147`
- 当前保留的唯一新增 Python 脚本：
  - `scripts/measure_connected_mcp_memory.py`
- `git status --short` 显示工作区较脏，包含较多未跟踪文件；不要默认把这些都当成本轮产出。
- `docs/handoff/` 目录是本轮新增，为本交接文档服务。

# 总结 bullet points
- 用户关注的是 `release` 真实场景，不是 debug benchmark。
- 当前文档已补充限定：现有脚本结果是“最小真实 stdio 握手基线”，不是包含 `--connect auto` 的完整日常使用态。
- 当前唯一保留的测量脚本是 `scripts/measure_connected_mcp_memory.py`。
- 这个脚本会启动 `target/release/nvim-mcp`，用最小 Python MCP 客户端真实握手，再测 RSS。
- `stdio` framing 已确认是 newline-delimited JSON，不是 `Content-Length`。
- `docs/mcp-memory-optimization-report-zh.md` 已存在，但可能还需要最后一次清理历史口径。
- 共享 `nvim-mcp` 进程已经按用户要求 kill 掉了。
- 当前没有活着的 `target/release/nvim-mcp` 进程。
- 仍有 9 个 headless 测试 `nvim` 存活，全部来自 `nvim-mcp` 仓库测试场景。
- 这 9 个连接不是 9 个 MCP 进程，而是曾经 1 个共享 `nvim-mcp` 内部维护的 9 条 Neovim 连接。
- 在 `mermaid_validator_rs` 场景里，Claude Code 看到的 Target 是 Neovim socket，不是 `nvim-mcp` 进程路径。
- kill `nvim-mcp` 不会自动清理这些 headless 测试 `nvim`。
- 若继续编码新功能，用户要求先确认再动手。

# 下一位 Agent 的第一步建议
第一步先做一件最简单但价值最高的事：复核那 9 个 headless 测试 `nvim` 是否仍存活，并根据用户意图决定是否 kill。

为什么先做这一步：

- 这是当前现场里最明确、最可执行、最容易继续推进的未完成事项。
- 它直接影响机器当前的实际内存占用，也能避免下一位 Agent 把这些残留误判成新的 MCP 问题。

做完这一步后如何决定后续分支：

- 如果用户要继续清理现场，就继续 kill 这 9 个 `nvim` 并复查。
- 如果用户转向“整理结论”，就回到 `docs/mcp-memory-optimization-report-zh.md` 做全文一致性清理。
- 如果用户转向“继续优化实现”，就进入 `src/main.rs` / `src/server/*` / `src/neovim/*` 开始设计共享 daemon 或连接回收方案。
