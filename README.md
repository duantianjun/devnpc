# devnpc

基于 GitLab 的企业级研发流程 AI 智能体(对标 CodeBuddy NPC)。

devnpc 监听 GitLab Issue/MR 中的 `@devnpc` 提及,在 CI 内自主完成"读上下文 → 改代码 → 提交 → 跟踪 CI → 修复失败"的闭环,并把执行轨迹沉淀为运维报告。

## 技术栈

| 层 | 选型 | 说明 |
|----|------|------|
| 语言 | Rust 2021 | 静态分发、零成本抽象、编译期错误检查 |
| 异步运行时 | tokio (full) | async/await + 超时 + 进程 |
| HTTP | reqwest (rustls) | 直连 OpenAI 兼容 Chat Completions API |
| LLM 抽象 | rig-core (P8 启用) | 多 provider 路由,P3 暂用 reqwest 直连 |
| GitLab API | reqwest + 自建 client | REST API v4 |
| 代码工具 | agent-file-tools + tree-sitter (P3.5) | AST 级代码感知,省 token |
| CLI | clap (derive) | `devnpc run / config / info` |
| 序列化 | serde + serde_json + serde_yaml | 配置 + 消息 + 工具参数 |
| 报告 | askama | 编译期 HTML 模板 |
| 日志 | tracing + tracing-subscriber | 结构化日志 |
| 错误 | thiserror + anyhow | 库层枚举 + CLI 层透明 |

## 架构

```
┌─────────────────────────────────────────────────────────────┐
│  CLI (main.rs)  devnpc run / config / info                  │
├─────────────────────────────────────────────────────────────┤
│  trigger  ← @devnpc 提及解析                                 │
│  memory   ← Git 仓库树 + Issue/PR/CI 记忆聚合                │
│  agent    ← ReAct 循环 (LLM ↔ Tool) + SOP 软约束            │
│  tools    ← 8 自建工具 + 5 AFT 工具 (P3.5)                   │
│  ci       ← Pipeline 失败 → 日志解析 → 修复闭环 (P4)         │
│  npc      ← Role/SOP 加载 + 单 NPC 执行器 (P6)              │
│  team     ← 多 NPC 编排 + GitLab 评论总线 (P6)              │
│  report   ← 轨迹采集 + HTML 生成 + 发布 (P7)                │
├─────────────────────────────────────────────────────────────┤
│  config   ← env > .devnpc.md > 默认值  (三层合并)            │
│  gitlab_api ← REST v4 客户端                                 │
│  git      ← GitOps (系统 git 命令封装)                       │
└─────────────────────────────────────────────────────────────┘
```

**核心设计决策(方案 C)**:
- **混合架构**:rig(LLM 抽象,P8 启用)+ 自建 ReAct 循环 + AFT(代码工具层)
- **SOP 双层约束**:soft 模式(偏离只警告)+ strict 模式(偏离即阻断,P6)
- **唯一副作用出口**:所有写操作走 `tools` 模块,便于审计与沙箱化
- **研发记忆**:任务执行前聚合 Git 仓库树 + 关键文件摘要 + Issue/PR/CI 历史,降低 LLM 上下文成本

## 模块说明

| 模块 | 职责 | 实现阶段 |
|------|------|----------|
| [src/config/](src/config/) | 三层配置合并(env > .devnpc.md > 默认) | P1 ✅ |
| [src/gitlab_api/](src/gitlab_api/) | GitLab REST v4 客户端(9 个 API 方法) | P1/P2 ✅ |
| [src/git/](src/git/) | GitOps 同步封装(`std::process::Command`) | P2 ✅ |
| [src/memory/](src/memory/) | 研发记忆聚合(Context + repo_index) | P2 ✅ |
| [src/agent/](src/agent/) | ReAct 循环 + LLM 客户端 + 消息类型 + prompt + SOP | P3 ✅ |
| [src/tools/](src/tools/) | 8 个自建工具 + ToolRegistry | P3 ✅ |
| [src/ci/](src/ci/) | CI 闭环控制器 + 日志解析 | P4 ⏳ |
| [src/trigger/](src/trigger/) | `@devnpc` 提及解析 | P5 ⏳ |
| [src/npc/](src/npc/) | Role/SOP 加载 + 单 NPC 执行器 | P6 ⏳ |
| [src/team/](src/team/) | 多 NPC 编排 + 评论总线 | P6 ⏳ |
| [src/report/](src/report/) | 轨迹采集 + HTML 报告 + 发布 | P7 ⏳ |

## 开发任务与实现情况

### ✅ P0 项目骨架
- [x] Cargo.toml 依赖与 bin/lib 双 target
- [x] `src/lib.rs` 模块树声明(11 个顶层模块)
- [x] `src/main.rs` CLI 骨架(`run` / `config` / `info` 子命令)
- [x] `src/error.rs` 统一错误类型 `DevnpcError`(thiserror)
- **验收**:构建通过,CLI 可运行

### ✅ P1 配置系统 + GitLab API 客户端(commit d853701)
- [x] `config/loader.rs`:三层合并(env > .devnpc.md > 默认值)
- [x] `config/devnpc_md.rs`:`.devnpc.md` Markdown 解析(sop_mode/forbidden_paths/required_checks/guidelines)
- [x] `config/env.rs`:环境变量类型解析(`DEVNPC_*` / `GITLAB_*` / `CI_PROJECT_ID`)
- [x] `gitlab_api/client.rs`:GitlabClient(9 个 API 方法)
  - get_issue / get_mr / create_mr
  - get_pipelines / get_recent_pipelines
  - get_issue_notes / get_mr_notes / create_mr_note
  - get_related_mrs
- [x] wiremock 集成测试
- [x] CLI `config` 命令集成
- **验收**:44 tests 通过

### ✅ P2 研发记忆
- [x] `memory/context.rs`:`Context` 聚合(仓库树/关键文件/Issue/PR/CI/提交)
- [x] `memory/repo_index.rs`:`build_repo_tree`(解析 `git ls-tree`,目录带尾斜杠展开)
- [x] `memory/repo_index.rs`:`select_key_files`(Cargo.toml/README/src/main.rs 摘要)
- [x] `git/ops.rs`:GitOps 同步封装(init/checkout/commit/diff/log/ls-tree)
- [x] `gitlab_api/client.rs`:扩展 `get_related_mrs` + `get_recent_pipelines`
- [x] `tokio::try_join!` 并行拉取 Git/GitLab 数据
- **验收**:62 tests 通过

### ✅ P3 ReAct Agent 循环(commit af70359)
- [x] `agent/message.rs`:OpenAI 兼容 `Message`/`ToolCall`/`ToolSchema`/`LlmResponse`
- [x] `agent/llm_client.rs`:reqwest 直连 Chat Completions(含 tool_calls 解析,ToolWrapper 包装)
- [x] `tools/mod.rs`:`Tool` trait(`parameters_schema`)+ `ToolRegistry`(`schemas`/`call`)
- [x] 8 个自建工具:
  - [x] `tools/file_io.rs`:read_file / write_file / list_files(path traversal 防护)
  - [x] `tools/git_tool.rs`:git_diff / git_commit
  - [x] `tools/shell.rs`:run_command(白名单/黑名单 + 超时)
  - [x] `tools/gitlab_tool.rs`:create_mr_note
  - [x] `tools/finish.rs`:finish(LLM 标记任务完成)
- [x] `agent/prompt.rs`:`build_initial_messages`(System 角色+规范+规则 / User 研发记忆+任务)
- [x] `agent/sop.rs`:`estimate_current_step` + `check_deviation`(软约束)
- [x] `agent/loop_.rs`:`ReactLoop::run`(LLM↔Tool 循环 + finish 检测 + 迭代上限 + Trajectory)
- **设计偏离**(已记录到 plan):
  - LLM 客户端用 reqwest 直连而非 rig-core(P8 引入 rig)
  - 5 个 AFT 工具(tree-sitter)推迟到 P3.5
  - SOP 仅 soft 模式(strict 留 P6)
- **验收**:110 tests 通过,clippy 零警告,release 构建成功

### ⏳ P3.5 AFT 代码感知工具(待开发)
- [ ] `tools/aft.rs`:view_symbol / edit_symbol / ast_replace / outline / search_symbols
- [ ] tree-sitter 多语言 grammar 集成
- **目标**:AST 级代码操作,省 token,提升大文件处理能力

### ⏳ P4 CI 闭环控制器(待开发)
- [ ] `ci/controller.rs`:Pipeline 失败 → 触发 Agent 修复 → 重试闭环
- [ ] `ci/log_parser.rs`:CI 日志根因定位(编译错误/测试失败/lint)
- [ ] 串联 `ReactLoop`,实现 `max_ci_retries` 限制
- **目标**:MR 提交后自动跟踪 CI,失败时自主修复并重试

### ⏳ P5 触发系统(待开发)
- [ ] `trigger/parser.rs`:解析 `@devnpc` 提及 + 任务描述
- [ ] GitLab Webhook 接入
- **目标**:Issue/MR 评论 `@devnpc 修复 xxx` 即触发任务

### ⏳ P6 NPC 角色系统 + 多 NPC 协同(待开发)
- [ ] `npc/role.rs`:Role 定义(dev/review/test/ops)
- [ ] `npc/sop.rs`:SOP 完整体系(strict 模式阻断)
- [ ] `npc/runner.rs`:单 NPC 执行器
- [ ] `team/orchestrator.rs`:多 NPC 编排
- [ ] `team/comm.rs`:GitLab 评论总线(NPC 间通信)
- **目标**:不同角色 NPC 协同完成复杂任务

### ⏳ P7 运维报告(待开发)
- [ ] `report/collector.rs`:Trajectory 采集
- [ ] `report/html.rs`:askama 模板渲染静态 HTML
- [ ] `report/publisher.rs`:发布到 Artifact / GitLab Pages
- **目标**:每次任务生成可查阅的运维报告

### ⏳ P8 模型路由(待开发)
- [ ] 引入 rig-core 替换 reqwest 直连
- [ ] 多 provider 路由(DeepSeek/OpenAI/Claude)
- [ ] 模型能力分级(简单任务用小模型,复杂任务用大模型)
- **目标**:成本优化,按任务复杂度选择模型

## 系统实现计划(对照表)

| 阶段 | 内容 | 状态 | 关键 commit | 测试数 |
|------|------|------|-------------|--------|
| P0 | 项目骨架 | ✅ 完成 | (初始提交) | - |
| P1 | 配置系统 + GitLab API | ✅ 完成 | d853701 | 44 |
| P2 | 研发记忆 | ✅ 完成 | (P2 系列) | 62 |
| P3 | ReAct Agent 循环 | ✅ 完成 | af70359 | 110 |
| P3.5 | AFT 代码感知工具 | ⏳ 待开发 | - | - |
| P4 | CI 闭环控制器 | ⏳ 待开发 | - | - |
| P5 | 触发系统 | ⏳ 待开发 | - | - |
| P6 | NPC 角色 + 多 NPC 协同 | ⏳ 待开发 | - | - |
| P7 | 运维报告 | ⏳ 待开发 | - | - |
| P8 | 模型路由(rig-core) | ⏳ 待开发 | - | - |

**当前进度**:P3 完成(110 tests,clippy 零警告,release 构建成功),已推送至 [GitHub](https://github.com/duantianjun/devnpc)。

## 环境变量

```bash
# LLM 配置 (必填)
export DEVNPC_API_KEY="sk-xxxxxxxx"
export DEVNPC_BASE_URL="https://api.deepseek.com/v1"
export DEVNPC_MODEL="deepseek-chat"

# GitLab 配置 (必填)
export GITLAB_URL="https://gitlab.example.com"
export GITLAB_TOKEN="glpat-xxxxxxxx"
export CI_PROJECT_ID="123"
```

## 使用方法

```bash
# 构建 release 版本
cargo build --release

# 查看版本信息
./target/release/devnpc info

# 查看当前配置(脱敏)
./target/release/devnpc config

# 运行 NPC 任务(CI 内调用)
./target/release/devnpc run --task "修复登录 bug"

# 干跑模式(不真正改码,冒烟测试用)
./target/release/devnpc run --dry-run
```

## 项目配置(.devnpc.md)

在仓库根目录放置 `.devnpc.md` 可覆盖默认约束:

```markdown
# NPC 项目规范

## 编码约定
- 禁止使用 unwrap()
- 必须处理所有 Result

## 禁止修改的路径
- src/config/
- Cargo.lock

## 提交前必须通过的检查
- cargo clippy -- -D warnings
- cargo test

## SOP 模式
soft  # 或 strict
```

## 开发约定

- **实现流程**:先写 plan 到 `docs/superpowers/plans/YYYY-MM-DD-devnpc-<phase>.md` → 确认设计偏离 → Inline 批量执行 + 检查点
- **TDD**:每个任务"写测试 → 验证失败 → 实现 → 验证通过 → commit"
- **验收门槛**:`cargo test --all` 全通过 + `cargo clippy --all-targets -- -D warnings` 零警告 + `cargo build --release` 成功
- **commit 规范**:`feat: <描述>` / `fix: <描述>` / `chore: <描述>` / `docs: <描述>`

## 仓库结构

```
devnpc/
├── src/
│   ├── agent/          # ReAct 循环 + LLM 客户端 + 消息 + prompt + SOP
│   ├── ci/             # CI 闭环控制器 (P4)
│   ├── config/         # 配置系统 (三层合并)
│   ├── git/            # GitOps 封装
│   ├── gitlab_api/     # GitLab REST v4 客户端
│   ├── memory/         # 研发记忆聚合
│   ├── npc/            # NPC 角色系统 (P6)
│   ├── report/         # 运维报告 (P7)
│   ├── team/           # 多 NPC 协同 (P6)
│   ├── tools/          # 8 自建工具 + ToolRegistry
│   ├── trigger/        # 事件触发 (P5)
│   ├── error.rs        # 统一错误类型
│   ├── lib.rs          # 模块树
│   └── main.rs         # CLI 入口
├── docs/superpowers/plans/  # 实现计划文档
├── Cargo.toml
└── README.md
```

## License

MIT
