# devnpc adk-rust 框架迁移设计文档

- **状态**: 待评审
- **创建日期**: 2026-08-02
- **基础设计**: [2026-08-01-devnpc-design.md](./2026-08-01-devnpc-design.md)
- **迁移目标**: 将自研 ReAct Agent + 工具系统 + NPC/Team 编排替换为 adk-rust 框架

## 摘要

devnpc 当前自研的 Agent 框架（ReAct 循环、LLM 客户端、工具系统、SOP 检测、NPC/Team 编排）将全面替换为 adk-rust 框架。保留业务逻辑层（GitLab API、Git 操作、CI 控制器、触发系统、记忆上下文、报告生成、配置加载），通过新增 `src/adapter/` 适配层桥接业务逻辑与 adk-rust 框架。项目升级至 Rust edition 2024。

## 核心决策汇总

| # | 决策项 | 选择 |
|---|---|---|
| M1 | 框架 | adk-rust v1.0.0 (zavora-ai) |
| M2 | Agent 架构 | LlmAgent + Runner (替代自建 ReAct 循环) |
| M3 | 工具系统 | FunctionTool 包装 (替代自建 Tool trait) |
| M4 | 多模型 | 启用 deepseek/openai/anthropic/gemini provider |
| M5 | 工作流编排 | graph 模块 (SequentialAgent/ParallelAgent) |
| M6 | 会话管理 | session 模块 (替代自建对话历史) |
| M7 | MCP 协议 | 启用 mcp 模块 (对接外部 MCP Server) |
| M8 | RAG | 启用 rag 模块 (检索增强) |
| M9 | 安全护栏 | 启用 guardrail 模块 (内容过滤) |
| M10 | 记忆管理 | 启用 memory 模块 (框架级会话记忆) |
| M11 | 代码沙箱 | 启用 code-exec 模块 (补充 run_command) |
| M12 | 评估测试 | 启用 eval 模块 (Agent 评测) |
| M13 | 状态管理 | 启用 session 模块 (会话持久化) |
| M14 | Callbacks | 用于 SOP 检测 + 轨迹记录 (替代自建) |
| M15 | Rust 版本 | 升级至 edition 2024 |

## 未启用模块

| 模块 | 原因 |
|---|---|
| realtime | 项目为 CI/CD/命令行模式，无需实时通信 |
| auth | 认证凭据通过 Config 系统管理 |
| telemetry | tracing/log 日志已满足当前监控需求 |

## 1. 架构变更

### 1.1 当前架构

```
┌───────────────────────────────────────────────┐
│ 第 1 层: agent/ (自建 ReAct 循环 + SOP 双层)   │
│  · loop_.rs: LLM ↔ Tool 循环 + 迭代上限       │
│  · prompt.rs: 系统提示词 + SOP 步骤注入        │
│  · llm_client.rs: reqwest 直连 OpenAI API      │
│  · message.rs: 消息类型定义                    │
├───────────────────────────────────────────────┤
│ 第 2 层: tools/ (自建工具系统)                 │
│  · mod.rs: Tool trait + ToolRegistry           │
│  · 8 个自建工具 + 5 个 AFT 工具               │
├───────────────────────────────────────────────┤
│ 第 3 层: npc/ (自建执行器)                     │
│  · runner.rs: 单 NPC 执行器                   │
│  · role.rs/sop.rs: 角色/SOP 加载              │
├───────────────────────────────────────────────┤
│ 第 4 层: team/ (自建编排器)                    │
│  · orchestrator.rs: 多 NPC 编排               │
└───────────────────────────────────────────────┘
```

### 1.2 目标架构

```
┌──────────────────────────────────────────────────────┐
│ 保留业务层 (不修改)                                    │
│  · config/  · gitlab_api/  · git/  · memory/         │
│  · ci/  · trigger/  · report/  · error.rs            │
├──────────────────────────────────────────────────────┤
│ adapter/ (新增适配层)                                  │
│  · tools.rs: 业务工具 → FunctionTool 包装            │
│  · callbacks.rs: before_tool_callback (SOP 检测)     │
│  · context.rs: 业务上下文 → Session 注入              │
│  · provider.rs: 多模型提供商配置                       │
├──────────────────────────────────────────────────────┤
│ adk-rust 框架层                                      │
│  · LlmAgent + Runner (替代 agent/loop_.rs)           │
│  · FunctionTool (替代 tools/Tool trait)              │
│  · Session (替代 agent/message.rs 对话管理)          │
│  · Callbacks (替代 agent/sop.rs 偏离检测)            │
│  · graph (替代 team/orchestrator.rs 编排)            │
│  · mcp/rag/guardrail/memory/code-exec/eval           │
├──────────────────────────────────────────────────────┤
│ 基础设施层                                            │
│  · tokio (异步运行时)                                 │
│  · reqwest (GitLab API 客户端,保留)                   │
│  · agent-file-tools (AFT 代码工具,保留)               │
└──────────────────────────────────────────────────────┘
```

### 1.3 核心执行流变化

**迁移前 (自建 ReAct 循环)**:
```
agent/loop_.rs: LLM 调用 → 工具执行 → 结果喂回 → 循环
  ↓ 自建 Tool trait 与 ToolRegistry
  ↓ 自建消息类型与管理
  ↓ 自建 SOP 偏离检测
```

**迁移后 (adk-rust Runner)**:
```
adk-rust Runner::run(LlmAgent):
  ├─ LlmAgent::before_tool_callback → SOP 检测 (adapter/callbacks.rs)
  ├─ FunctionTool::call → 业务工具执行 (adapter/tools.rs)
  ├─ Session → 对话管理 (adapter/context.rs)
  └─ Trajectory 记录 → report 模块消费
```

## 2. 模块映射

### 2.1 删除模块 (由 adk-rust 替代)

| 当前文件 | 替代方案 | 说明 |
|---|---|---|
| `src/agent/loop_.rs` | adk-rust Runner | LlmAgent + Runner 替代自建 ReAct 循环 |
| `src/agent/llm_client.rs` | adk-rust models | 通过 `deepseek`/`openai` 等 provider feature 替代 |
| `src/agent/message.rs` | adk-rust session | Session 管理对话状态 |
| `src/agent/prompt.rs` | adk-rust LlmAgent | 系统提示词在 LlmAgent::system_prompt 中配置 |
| `src/agent/sop.rs` | adk-rust callbacks | Callbacks 实现 before_tool_callback |
| `src/agent/mod.rs` | — | 模块入口移除 |
| `src/tools/finish.rs` | adk-rust 内置 | Finish 信号由 Runner 自动处理 |
| `src/npc/mod.rs` | adk-rust runner | 单 NPC 执行由 Runner + LlmAgent 替代 |
| `src/npc/runner.rs` | adk-rust runner | 同上 |
| `src/npc/role.rs` | adk-rust config | 角色配置通过 LlmAgent 参数配置 |
| `src/npc/sop.rs` | adk-rust callbacks | SOP 通过 Callbacks 实现 |
| `src/team/mod.rs` | adk-rust graph | SequentialAgent/ParallelAgent 替代 |
| `src/team/orchestrator.rs` | adk-rust graph | 同上 |
| `src/team/comm.rs` | adk-rust 评论总线 | 通信仍通过 GitLab 评论，但编排用 graph |

### 2.2 保留模块 (业务逻辑)

| 模块 | 说明 | 修改方式 |
|---|---|---|
| `src/config/` | 配置系统 | 增加 provider 字段，适配多模型 |
| `src/gitlab_api/` | GitLab REST API 客户端 | 不修改 |
| `src/git/` | Git 操作封装 | 不修改 |
| `src/memory/` | 研发记忆上下文构建 | 不修改，通过 Session 注入 |
| `src/ci/` | CI 闭环控制器 | 调用新的 Runner 接口 |
| `src/trigger/` | 事件触发解析 | 不修改 |
| `src/report/` | 报告生成 | 通过 Callbacks 收集轨迹数据 |
| `src/error.rs` | 统一错误类型 | 增加 adk-rust 相关错误变体 |

### 2.3 新增模块 (adapter/)

| 文件 | 职责 |
|---|---|
| `src/adapter/mod.rs` | 适配层模块入口，重新导出关键类型 |
| `src/adapter/tools.rs` | 将业务工具包装为 FunctionTool |
| `src/adapter/callbacks.rs` | 实现 Callbacks trait (SOP 检测 + 轨迹记录) |
| `src/adapter/context.rs` | 将业务 Context 注入 Session |
| `src/adapter/provider.rs` | 根据 Config 创建多模型 Provider |

### 2.4 修改模块

| 文件 | 修改内容 |
|---|---|
| `src/main.rs` | 使用 Runner + LlmAgent 替代 ReactLoop |
| `src/lib.rs` | 模块声明调整（移除 agent/npc/team，新增 adapter） |
| `src/config/mod.rs` | LlmConfig 增加 provider 字段 |
| `src/ci/controller.rs` | 调用 adk-rust Runner 执行修复 |
| `src/report/collector.rs` | 从 Callbacks 收集轨迹数据 |

## 3. adk-rust 功能模块使用场景

### 3.1 LlmAgent + Runner (核心执行)

```rust
// 迁移后 main.rs 核心逻辑
use adk_rust::agent::LlmAgent;
use adk_rust::runner::Runner;
use adk_rust::tool::FunctionTool;

let tools = adapter::tools::create_all_tools(config)?;
let callbacks = adapter::callbacks::DevnpcCallbacks::new(config, reporter);

let agent = LlmAgent::builder()
    .model(adapter::provider::create_model(&config.llm)?)
    .system_prompt(build_system_prompt(task, context))
    .tools(tools)
    .before_tool_callback(callbacks.before_tool_callback())
    .max_iterations(config.limits.max_iterations)
    .build();

let mut session = Runner::new(agent).new_session();
// 注入业务上下文
adapter::context::inject_context(&mut session, context)?;

let result = Runner::new(agent).run(session).await?;
```

### 3.2 FunctionTool (工具包装)

```rust
// adapter/tools.rs
use adk_rust::tool::FunctionTool;

pub fn create_read_file_tool() -> FunctionTool {
    FunctionTool::new("read_file", |params: ReadFileParams| async move {
        // 调用现有的 file_io::read_file 逻辑
        tools::file_io::read_file(&params.path).await
    })
    .with_description("读取文件内容，路径限制在 workspace 内")
    .with_parameter("path", "文件路径")
}

pub fn create_all_tools(config: &Config) -> Vec<FunctionTool> {
    vec![
        create_read_file_tool(),
        create_write_file_tool(),
        create_run_command_tool(config),
        create_git_tool(config),
        create_gitlab_tool(config),
        // ... AFT 工具等
    ]
}
```

### 3.3 Callbacks (SOP 检测 + 轨迹记录)

```rust
// adapter/callbacks.rs
use adk_rust::agent::callback::BeforeToolCallback;

pub struct DevnpcCallbacks {
    sop: Option<Sop>,
    reporter: Arc<Reporter>,
}

impl BeforeToolCallback for DevnpcCallbacks {
    async fn before_tool_call(
        &self,
        context: &ToolCallContext,
    ) -> Result<(), ToolCallError> {
        // SOP 偏离检测
        if let Some(sop) = &self.sop {
            check_sop_deviation(sop, context)?;
        }
        // 轨迹记录
        self.reporter.record_tool_call(context).await;
        Ok(())
    }
}
```

### 3.4 graph (多 NPC 编排)

```rust
// 替代 team/orchestrator.rs
use adk_rust::graph::{SequentialAgent, ParallelAgent};

// PM 拆解需求 → 串行执行
let pm_agent = create_npc_agent("pm", &config, &sops["requirement-decompose"]);
let dev_agent = create_npc_agent("developer", &config, &sops["feature"]);
let test_agent = create_npc_agent("tester", &config, &sops["test-gen"]);

// 开发 + 测试并行执行
let parallel = ParallelAgent::new(vec![dev_agent, test_agent]);
// 先 PM 拆解，再并行开发/测试
let pipeline = SequentialAgent::new(vec![pm_agent, parallel]);

let result = Runner::new(pipeline).run(session).await?;
```

### 3.5 Session (对话管理)

```rust
// adapter/context.rs
use adk_rust::session::Session;

pub fn inject_context(session: &mut Session, ctx: &Context) {
    session.set("repo_tree", &ctx.repo_tree);
    session.set("issue", &ctx.issue);
    session.set("ci_failures", &ctx.ci_failures);
    session.set("project_config", &ctx.project_config);
}
```

### 3.6 guardrail (安全过滤)

```rust
// 配置 ContentGuardrail 过滤 LLM 输入/输出
use adk_rust::guardrail::{ContentGuardrail, GuardrailConfig};

let guardrail = ContentGuardrail::builder()
    .block_path_traversal()  // 阻止路径穿越
    .block_command_injection()  // 阻止命令注入
    .max_input_tokens(8000)  // 输入 Token 上限
    .build();
```

### 3.7 memory (会话记忆)

```rust
// 启用框架级记忆管理
use adk_rust::memory::{SessionMemory, MemoryConfig};

let memory = SessionMemory::new(MemoryConfig {
    short_term_size: 20,    // 保留最近 20 轮对话
    enable_summary: true,   // 超长时自动摘要
    persist_path: None,     // 不持久化 (CI 环境无状态)
});
```

### 3.8 code-exec (代码沙箱)

```rust
// 作为 run_command 的补充，用于安全执行代码片段
use adk_rust::code_exec::CodeExecutor;

let executor = CodeExecutor::builder()
    .allowed_languages(vec!["rust", "python", "shell"])
    .timeout(Duration::from_secs(30))
    .max_output_size(1024 * 100)  // 100KB
    .build();
```

### 3.9 eval (评测)

```rust
// 用于验收测试场景
use adk_rust::eval::Evaluator;

let evaluator = Evaluator::new()
    .add_criterion("ci_passed", |result| {
        result.metadata.get("ci_status") == Some(&"success".to_string())
    })
    .add_criterion("code_compiles", |result| {
        result.tool_calls.iter().any(|tc| tc.name == "run_command" && tc.success)
    });
```

## 4. 依赖变更

### 4.1 Cargo.toml 变更

**移除的依赖**:
- `futures` (adk-rust 内部使用)
- `async-trait` (adk-rust 内部使用)
- `thiserror` (可用 adk-rust 错误类型或保留)
- `anyhow` (可用 adk-rust 错误类型或保留)
- `dotenvy` (Config 系统保留，但可改用 adk-rust 配置)

**保留的依赖**:
- `tokio` (异步运行时，adk-rust 依赖)
- `reqwest` (GitLab API 客户端)
- `serde` / `serde_json` / `serde_yaml` (序列化)
- `clap` (CLI 参数解析)
- `tracing` / `tracing-subscriber` (日志)
- `agent-file-tools` / `tree-sitter` / `tree-sitter-rust` (AFT 工具)
- `url` / `chrono` / `regex` (工具库)
- `tokio-test` / `mockall` / `tempfile` / `wiremock` (测试依赖)

**新增的依赖**:
```toml
adk-rust = { version = "1", features = [
    "deepseek", "openai", "anthropic", "gemini",  # 模型提供商
    "agent", "runner", "session", "tool",          # 核心框架
    "mcp", "graph", "rag", "guardrail",           # 功能模块
    "memory", "code-exec", "eval"                 # 功能模块
] }
```

### 4.2 Rust 版本升级

```toml
[package]
edition = "2024"  # 从 2021 升级
```

## 5. 目录结构变更

```
devnpc/
├── Cargo.toml
├── Dockerfile
├── .gitlab-ci.yml.example
├── npc-config/
│   ├── roles/{developer,tester,pm}.yml
│   ├── sops/{bugfix,feature,test-gen}.yml
│   └── teams/feature-team.yml
└── src/
    ├── main.rs                    ← 修改: 使用 Runner + LlmAgent
    ├── lib.rs                     ← 修改: 模块声明
    ├── error.rs                   ← 微调: 增加 adk 错误变体
    ├── adapter/                   ← 新增: 适配层
    │   ├── mod.rs
    │   ├── tools.rs               ← FunctionTool 包装
    │   ├── callbacks.rs           ← SOP 检测 + 轨迹记录
    │   ├── context.rs             ← Session 注入
    │   └── provider.rs            ← 多模型提供商
    ├── config/                    ← 保留,微调
    ├── gitlab_api/                ← 保留,不修改
    ├── git/                       ← 保留,不修改
    ├── memory/                    ← 保留,不修改
    ├── ci/                        ← 保留,调用新接口
    ├── trigger/                   ← 保留,不修改
    ├── report/                    ← 保留,从 Callbacks 采集
    ├── agent/                     ← 删除: 由 adk-rust 替代
    ├── tools/                     ← 保留(工具逻辑),删除 mod.rs 自建框架
    ├── npc/                       ← 删除: 由 adk-rust 替代
    └── team/                      ← 删除: 由 adk-rust graph 替代
```

## 6. 实施阶段

### 6.1 阶段 A: 基础设施准备

- [ ] 升级 Cargo.toml: edition 2024 + 新增 adk-rust 依赖
- [ ] 创建 `src/adapter/` 目录结构
- [ ] 编译验证: `cargo build` 通过

### 6.2 阶段 B: 工具系统迁移

- [ ] 在 `adapter/tools.rs` 中将现有工具包装为 FunctionTool
- [ ] 保留 `src/tools/` 下各工具的实现逻辑
- [ ] 移除 `src/tools/mod.rs` 自建 Tool trait + ToolRegistry
- [ ] 测试验证: 所有工具独立调用正常

### 6.3 阶段 C: 核心执行流迁移

- [ ] 实现 `adapter/provider.rs` 多模型提供商
- [ ] 实现 `adapter/callbacks.rs` SOP 检测 + 轨迹记录
- [ ] 实现 `adapter/context.rs` 业务上下文注入 Session
- [ ] 修改 `src/main.rs` 使用 Runner + LlmAgent
- [ ] 删除 `src/agent/` 目录
- [ ] 测试验证: 单 Agent 执行闭环通过

### 6.4 阶段 D: 多 NPC 编排迁移

- [ ] 使用 adk-rust graph 模块替代 `src/team/`
- [ ] 删除 `src/team/` 目录
- [ ] 删除 `src/npc/` 目录
- [ ] 测试验证: 多 NPC 协作流程通过

### 6.5 阶段 E: 功能模块集成

- [ ] 集成 guardrail (安全过滤)
- [ ] 集成 memory (会话记忆)
- [ ] 集成 code-exec (代码沙箱)
- [ ] 集成 mcp (MCP Server 对接)
- [ ] 集成 rag (检索增强)
- [ ] 集成 eval (评测)
- [ ] 集成 session (状态持久化)

### 6.6 阶段 F: 集成测试

- [ ] 更新 CI 适配层接口
- [ ] 更新 report 轨迹采集
- [ ] 更新测试用例
- [ ] 端到端冒烟测试通过
- [ ] `cargo clippy -D warnings` 通过

## 7. 风险与应对

| 风险 | 应对 |
|---|---|
| adk-rust API 不稳定 | 锁定 v1.0.0 版本，adapter 层隔离变化 |
| Rust edition 2024 兼容性问题 | 先在分支测试编译，逐步修复 |
| FunctionTool 参数序列化差异 | 统一使用 serde，测试覆盖所有工具 |
| 原有 165 个测试用例需要适配 | 保留业务逻辑测试，替换框架相关测试 |
| CI 环境需要 adk-rust 编译通过 | 提前验证 Dockerfile 构建 |
| edition 2024 与 tree-sitter 等依赖兼容性 | 升级 tree-sitter 等依赖到兼容版本 |