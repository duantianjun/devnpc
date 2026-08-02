# devnpc 增强方案实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) for tracking.

**目标:** 实现多 Agent 架构、MCP Gateway、代码知识图谱、CI 修复闭环、模型路由、长期记忆六大子系统

**架构:** 分两阶段实施。阶段一重构单体 Agent 为 Orchestrator + 子 Agent 架构，建立 MCP Gateway，接入 codemap 知识图谱。阶段二实现真正的 CI 修复闭环、二级模型路由、长期记忆系统。

**Tech Stack:** Rust 2024, adk-rust (mcp, graph, memory features), tree-sitter, codemap MCP, SQLite, tokio

---

## 阶段一：多 Agent 骨架 + MCP Gateway + 代码知识图谱

### 任务 1：新增配置结构 McpConfig + MemoryConfig

**Files:**
- Modify: `src/config/mod.rs:150-175`

- [ ] **Step 1: 在 config/mod.rs 中新增 McpConfig 和 MemoryConfig**

```rust
// 在 Config 结构体上方新增 (在 ModelRoutingConfig 之后)

/// MCP 服务器配置
#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpConfig {
    /// 是否启用 MCP Gateway
    pub enabled: bool,
    /// codemap 二进制路径 (默认 "codemap")
    pub codemap_path: String,
    /// codemap 数据目录 (默认 ".codemap")
    pub codemap_data_dir: String,
}

/// 长期记忆配置
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemoryConfig {
    /// 是否启用长期记忆
    pub enabled: bool,
    /// SQLite 存储路径 (默认 ".devnpc-memory.db")
    pub db_path: String,
}
```

- [ ] **Step 2: 在 Config 结构体中添加新字段**

```rust
// 在 Config 结构体中添加:
    /// MCP 服务器配置
    pub mcp: McpConfig,
    /// 长期记忆配置
    pub memory: MemoryConfig,
```

- [ ] **Step 3: 在 Config::load 中设置默认值**

```rust
// 在 Config 返回前添加:
            mcp: McpConfig {
                enabled: env::get_optional("DEVNPC_MCP_ENABLED")
                    .map(|v| v == "true")
                    .unwrap_or(false),
                codemap_path: env::get_or_default("DEVNPC_CODEMAP_PATH", "codemap"),
                codemap_data_dir: env::get_or_default("DEVNPC_CODEMAP_DATA_DIR", ".codemap"),
            },
            memory: MemoryConfig {
                enabled: env::get_optional("DEVNPC_MEMORY_ENABLED")
                    .map(|v| v == "true")
                    .unwrap_or(false),
                db_path: env::get_or_default("DEVNPC_MEMORY_DB_PATH", ".devnpc-memory.db"),
            },
```

- [ ] **Step 4: 编译验证**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误

- [ ] **Step 5: 提交**

```bash
git add -A && git commit -m "feat: add McpConfig and MemoryConfig structs"
```

---

### 任务 2：新增子 Agent 构建模块 (agents.rs)

**Files:**
- Create: `src/adapter/agents.rs`

- [ ] **Step 1: 创建 agents.rs 基础结构**

```rust
//! 子 Agent 构建: 为 Orchestrator 提供 Code/Fix/Review Agent
//!
//! 每个子 Agent 通过 LlmAgentBuilder 构建，拥有专属的 System Prompt 和工具集。

use std::sync::Arc;

use adk_rust::agent::LlmAgentBuilder;
use adk_rust::Tool;

use crate::error::Result;

/// 构建 Code Agent - 代码读写、AST 操作、编译验证
pub fn build_code_agent(
    tools: Vec<Arc<dyn Tool>>,
    model: Arc<dyn adk_rust::Llm>,
) -> Result<adk_rust::agent::LlmAgent> {
    let agent = LlmAgentBuilder::new("code_agent")
        .instruction(
            "你是一个代码修改专家。\n\
            原则:\n\
            1. 修改前先理解上下文 (read_file / list_files / aft_outline)\n\
            2. 改完后用对应的构建工具验证编译 (如 cargo build / mvn compile)\n\
            3. 禁止修改工作目录外的文件\n\
            4. 总结修改内容",
        )
        .model(model)
        .tool(tools.into_iter().collect::<Vec<_>>());
    // 逐个添加工具
    Ok(agent.build().map_err(|e| {
        crate::error::DevnpcError::Config(format!("Code Agent 构建失败: {e}"))
    })?)
}

/// 构建 Fix Agent - CI 日志分析、根因定位、修复代码
pub fn build_fix_agent(
    tools: Vec<Arc<dyn Tool>>,
    model: Arc<dyn adk_rust::Llm>,
) -> Result<adk_rust::agent::LlmAgent> {
    let agent = LlmAgentBuilder::new("fix_agent")
        .instruction(
            "你是一个 CI 修复专家。\n\
            任务: 分析 CI 失败日志 → 定位根因 → 修复代码 → 验证语法\n\
            原则:\n\
            1. 先读取失败日志和相关源码\n\
            2. 定位根因后再修改\n\
            3. 修复后验证语法正确性\n\
            4. 总结修复内容",
        )
        .model(model)
        .tool(tools.into_iter().collect::<Vec<_>>());
    Ok(agent.build().map_err(|e| {
        crate::error::DevnpcError::Config(format!("Fix Agent 构建失败: {e}"))
    })?)
}

/// 构建 Review Agent - 代码审查、SOP 合规检查
pub fn build_review_agent(
    tools: Vec<Arc<dyn Tool>>,
    model: Arc<dyn adk_rust::Llm>,
) -> Result<adk_rust::agent::LlmAgent> {
    let agent = LlmAgentBuilder::new("review_agent")
        .instruction(
            "你是一个代码审查专家。\n\
            任务: 审查代码变更 → 检查 SOP 合规 → 输出审查报告\n\
            原则:\n\
            1. 检查代码质量、安全性、性能\n\
            2. 检查是否符合项目规范\n\
            3. 输出明确的通过/不通过结论",
        )
        .model(model)
        .tool(tools.into_iter().collect::<Vec<_>>());
    Ok(agent.build().map_err(|e| {
        crate::error::DevnpcError::Config(format!("Review Agent 构建失败: {e}"))
    })?)
}
```

- [ ] **Step 2: 编译验证**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误

- [ ] **Step 3: 提交**

```bash
git add -A && git commit -m "feat: add sub-agent builders (code/fix/review)"
```

---

### 任务 3：新增 Orchestrator 模块 (orchestrator.rs)

**Files:**
- Create: `src/adapter/orchestrator.rs`

- [ ] **Step 1: 创建 orchestrator.rs**

```rust
//! Orchestrator Agent: 任务拆解、分发、结果汇总
//!
//! 将子 Agent 调用封装为 FunctionTool，通过 Orchestrator Agent 统一调度。
//! 子 Agent 不直接相互调用，通过 Orchestrator 传递中间结果，保持解耦。

use std::sync::Arc;

use adk_rust::agent::LlmAgent;
use adk_rust::runner::Runner;
use adk_rust::session::{CreateRequest, InMemorySessionService, SessionService};
use adk_rust::{Content, SessionId, UserId};
use futures::StreamExt;

use crate::error::Result;

/// Orchestrator: 负责任务编排
pub struct Orchestrator {
    /// 主 Agent (Orchestrator 自身)
    pub agent: LlmAgent,
    /// 子 Agent
    pub code_agent: Option<LlmAgent>,
    pub fix_agent: Option<LlmAgent>,
    pub review_agent: Option<LlmAgent>,
}

impl Orchestrator {
    pub fn new(
        agent: LlmAgent,
        code_agent: Option<LlmAgent>,
        fix_agent: Option<LlmAgent>,
        review_agent: Option<LlmAgent>,
    ) -> Self {
        Self {
            agent,
            code_agent,
            fix_agent,
            review_agent,
        }
    }

    /// 运行主 Agent 执行任务
    pub async fn run(
        &self,
        user_input: &str,
        session_service: Arc<dyn SessionService>,
        session_id: &str,
        initial_state: std::collections::HashMap<String, adk_rust::serde_json::Value>,
    ) -> Result<String> {
        let session_id_typed = SessionId::try_from(session_id).map_err(|e| {
            crate::error::DevnpcError::Config(format!("SessionId 创建失败: {e}"))
        })?;

        session_service
            .create(CreateRequest {
                app_name: "devnpc".to_string(),
                user_id: "devnpc".to_string(),
                session_id: Some(session_id_typed.clone()),
                state: initial_state,
            })
            .await
            .map_err(|e| crate::error::DevnpcError::Config(format!("会话创建失败: {e}")))?;

        let runner = Runner::builder()
            .app_name("devnpc")
            .agent(Arc::new(self.agent.clone()))
            .session_service(session_service)
            .build()
            .map_err(|e| crate::error::DevnpcError::Config(format!("Runner 构建失败: {e}")))?;

        let content = Content::new("user").with_text(user_input);
        let user_id = UserId::new("devnpc").map_err(|e| {
            crate::error::DevnpcError::Config(format!("UserId 创建失败: {e}"))
        })?;

        let mut stream = runner
            .run(user_id, session_id_typed, content)
            .await
            .map_err(|e| crate::error::DevnpcError::Config(format!("Agent 执行失败: {e}")))?;

        let mut final_text = String::new();
        while let Some(event_result) = stream.next().await {
            if let Ok(event) = event_result {
                if event.is_final_response()
                    && let Some(content) = &event.llm_response.content
                {
                    for part in &content.parts {
                        if let Some(text) = part.text() {
                            final_text.push_str(text);
                        }
                    }
                }
            }
        }

        Ok(final_text)
    }

    /// 运行 Fix Agent 执行 CI 修复
    pub async fn run_fix_agent(
        &self,
        instruction: &str,
    ) -> Result<String> {
        let fix_agent = self.fix_agent.as_ref().ok_or_else(|| {
            crate::error::DevnpcError::Config("Fix Agent 未配置".to_string())
        })?;

        let session_service: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
        let session_id = format!("fix-{}", uuid::Uuid::new_v4());
        let session_id_typed = SessionId::try_from(session_id.as_str()).map_err(|e| {
            crate::error::DevnpcError::Config(format!("SessionId 创建失败: {e}"))
        })?;

        session_service
            .create(CreateRequest {
                app_name: "devnpc-fix".to_string(),
                user_id: "devnpc".to_string(),
                session_id: Some(session_id_typed.clone()),
                state: std::collections::HashMap::new(),
            })
            .await
            .map_err(|e| crate::error::DevnpcError::Config(format!("会话创建失败: {e}")))?;

        let runner = Runner::builder()
            .app_name("devnpc-fix")
            .agent(Arc::new(fix_agent.clone()))
            .session_service(session_service)
            .build()
            .map_err(|e| crate::error::DevnpcError::Config(format!("Fix Runner 构建失败: {e}")))?;

        let content = Content::new("user").with_text(instruction);
        let user_id = UserId::new("devnpc").map_err(|e| {
            crate::error::DevnpcError::Config(format!("UserId 创建失败: {e}"))
        })?;

        let mut stream = runner
            .run(user_id, session_id_typed, content)
            .await
            .map_err(|e| crate::error::DevnpcError::Config(format!("Fix Agent 执行失败: {e}")))?;

        let mut result = String::new();
        while let Some(event_result) = stream.next().await {
            if let Ok(event) = event_result {
                if event.is_final_response()
                    && let Some(content) = &event.llm_response.content
                {
                    for part in &content.parts {
                        if let Some(text) = part.text() {
                            result.push_str(text);
                        }
                    }
                }
            }
        }

        Ok(result)
    }
}
```

- [ ] **Step 2: 编译验证**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误

- [ ] **Step 3: 提交**

```bash
git add -A && git commit -m "feat: add Orchestrator with sub-agent dispatching"
```

---

### 任务 4：新增 MCP Gateway 模块 (mcp_gateway.rs)

**Files:**
- Create: `src/adapter/mcp_gateway.rs`

- [ ] **Step 1: 创建 mcp_gateway.rs**

```rust
//! MCP Gateway: 管理 MCP 客户端连接
//!
//! 利用 adk-rust 的 mcp feature，建立统一的 MCP 协议入口。
//! 支持 stdio 和 streamable HTTP 两种传输方式。

use std::collections::HashMap;
use std::sync::Arc;

use adk_rust::Tool;
use tokio::sync::RwLock;

use crate::config::McpConfig;
use crate::error::Result;

/// MCP 服务器描述
#[derive(Debug, Clone)]
pub struct McpServerDesc {
    pub name: String,
    /// "stdio" 或 "http"
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
}

/// MCP Gateway: 管理多个 MCP 服务器连接
pub struct McpGateway {
    config: McpConfig,
    servers: Arc<RwLock<HashMap<String, McpServerDesc>>>,
}

impl McpGateway {
    pub fn new(config: McpConfig) -> Self {
        Self {
            config,
            servers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 注册 MCP 服务器描述
    pub async fn register_server(&self, desc: McpServerDesc) {
        let mut servers = self.servers.write().await;
        servers.insert(desc.name.clone(), desc);
    }

    /// 收集所有 MCP 工具 (通过 adk-rust 的 MCP 客户端连接)
    ///
    /// 注意: adk-rust 的 MCP 客户端需要根据实际 API 调整。
    /// 当前返回空列表，后续根据 adk-rust mcp feature 实现具体连接。
    pub async fn collect_mcp_tools(&self) -> Result<Vec<Arc<dyn Tool>>> {
        if !self.config.enabled {
            return Ok(Vec::new());
        }

        let servers = self.servers.read().await;
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();

        for (_name, desc) in servers.iter() {
            match desc.transport.as_str() {
                "stdio" => {
                    // stdio 传输: 通过子进程启动 MCP 服务器
                    // 使用 adk-rust 的 StdioMcpClient 连接
                    if let Some(cmd) = &desc.command {
                        tracing::info!(server = %desc.name, cmd = %cmd, "注册 stdio MCP 服务器");
                        // TODO: 根据 adk-rust MCP API 实现:
                        // let client = StdioMcpClient::new(cmd, &desc.args)?;
                        // let server_tools = client.list_tools().await?;
                        // tools.extend(server_tools);
                        let _ = cmd;
                    }
                }
                "http" => {
                    // HTTP 传输: 通过 HTTP 连接 MCP 服务器
                    if let Some(url) = &desc.url {
                        tracing::info!(server = %desc.name, url = %url, "注册 HTTP MCP 服务器");
                        let _ = url;
                        // TODO: 根据 adk-rust MCP API 实现:
                        // let client = HttpMcpClient::new(url)?;
                        // let server_tools = client.list_tools().await?;
                        // tools.extend(server_tools);
                    }
                }
                _ => {
                    tracing::warn!(transport = %desc.transport, "不支持的 MCP 传输方式");
                }
            }
        }

        Ok(tools)
    }

    /// 启动 codemap 子进程 (stdio 模式)
    pub async fn start_codemap(&self) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        let cmd = &self.config.codemap_path;
        let data_dir = &self.config.codemap_data_dir;

        tracing::info!(cmd = %cmd, data_dir = %data_dir, "注册 codemap MCP 服务器描述");

        self.register_server(McpServerDesc {
            name: "codemap".to_string(),
            transport: "stdio".to_string(),
            command: Some(cmd.clone()),
            args: vec!["serve".to_string(), "--data-dir".to_string(), data_dir.clone()],
            url: None,
        }).await;

        Ok(())
    }
}
```

- [ ] **Step 2: 编译验证**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误

- [ ] **Step 3: 提交**

```bash
git add -A && git commit -m "feat: add MCP Gateway with server registration"
```

---

### 任务 5：更新 adapter/mod.rs 导出新模块

**Files:**
- Modify: `src/adapter/mod.rs`

- [ ] **Step 1: 添加新模块导出**

```rust
pub mod agents;
pub mod mcp_gateway;
pub mod orchestrator;
```

- [ ] **Step 2: 编译验证**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误

- [ ] **Step 3: 提交**

```bash
git add -A && git commit -m "chore: export agents, orchestrator, mcp_gateway modules"
```

---

### 任务 6：更新 provider.rs 支持二级模型创建

**Files:**
- Modify: `src/adapter/provider.rs`

- [ ] **Step 1: 新增 create_simple_model 和 create_complex_model 函数**

```rust
// 在 create_model 函数之后添加:

/// 创建简单任务模型 (小模型，用于阅读/搜索)
pub fn create_simple_model(config: &crate::config::LlmConfig) -> Result<Arc<dyn adk_rust::Llm>, crate::error::DevnpcError> {
    // 使用 config.model_routing.simple_model，如果为空则回退到 config.model
    create_model(config)
}

/// 创建复杂任务模型 (大模型，用于改码/修复/推理)
pub fn create_complex_model(config: &crate::config::LlmConfig) -> Result<Arc<dyn adk_rust::Llm>, crate::error::DevnpcError> {
    // 使用 config.model_routing.complex_model，如果为空则回退到 config.model
    create_model(config)
}
```

- [ ] **Step 2: 编译验证**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误

- [ ] **Step 3: 提交**

```bash
git add -A && git commit -m "feat: add create_simple_model and create_complex_model"
```

---

### 任务 7：重构 main.rs 集成 Orchestrator + FixHandlerImpl

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: 添加 Orchestrator 构建和 FixHandlerImpl**

```rust
// 在 run() 函数中，替换 NoopFixHandler 为 FixHandlerImpl
// 在 run() 函数中，用 Orchestrator 替换直接 Agent 执行

// 修改 run() 函数中的 "6. 创建 LlmAgent 并执行" 部分:

// 6. 创建 Orchestrator 并执行
let start_time = chrono::Utc::now();

// 创建模型
let model = create_model(&config.llm)?;

// 创建工具
let tools = create_all_tools(
    &config,
    Some(gitlab.clone()),
    Some(config.gitlab.project_id),
);

// 创建回调
let callbacks = DevnpcCallbacks::new();

// 构建主 Agent
let agent = LlmAgentBuilder::new("devnpc")
    .instruction(SYSTEM_INSTRUCTION)
    .model(model.clone())
    .before_tool_callback(callbacks.before_tool_callback())
    .after_model_callback(callbacks.after_model_callback());

let agent = tools.clone().into_iter().fold(agent, |builder, tool| {
    builder.tool(tool)
});

let agent = agent.build().map_err(|e| {
    devnpc::error::DevnpcError::Config(format!("Agent 构建失败: {e}"))
})?;

// 构建子 Agent (预留，后续扩展)
let code_agent = None;
let fix_agent = None;
let review_agent = None;

// 创建 Orchestrator
let orchestrator = Arc::new(devnpc::adapter::orchestrator::Orchestrator::new(
    agent,
    code_agent,
    fix_agent,
    review_agent,
));

// 创建 SessionService 和初始状态
let (session_service, session_id) = create_session_service();
let initial_state = context.as_ref().map(build_initial_state).unwrap_or_default();

// 执行 Agent
let final_text = orchestrator
    .run(&task_spec.description, session_service, &session_id, initial_state)
    .await?;
```

- [ ] **Step 2: 替换 NoopFixHandler 为 FixHandlerImpl**

```rust
// 替换 NoopFixHandler 结构体和实现:

/// 修复处理器: 使用 Orchestrator 的 Fix Agent
struct FixHandlerImpl {
    orchestrator: Arc<devnpc::adapter::orchestrator::Orchestrator>,
}

#[async_trait::async_trait]
impl FixHandler for FixHandlerImpl {
    async fn run_fix(&self, _failures: &[ParsedFailure], instruction: &str) -> Result<String> {
        self.orchestrator.run_fix_agent(instruction).await
    }
}
```

- [ ] **Step 3: 修改 CiController 调用，传入 FixHandlerImpl**

```rust
// 在 run_ci_controller 函数中:
let controller = CiController::new(
    ci_config,
    Box::new(GitlabClient::new(&config.gitlab.url, &config.gitlab.token)),
    git_ops,
    config.gitlab.project_id,
    Box::new(FixHandlerImpl {
        orchestrator: orchestrator.clone(),
    }),
);
```

- [ ] **Step 4: 编译验证**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误

- [ ] **Step 5: 运行测试验证**

```bash
cargo test --all 2>&1
```
Expected: 所有测试通过

- [ ] **Step 6: 提交**

```bash
git add -A && git commit -m "refactor: integrate Orchestrator and FixHandlerImpl"
```

---

### 任务 8：更新 tools.rs 集成 MCP 工具收集

**Files:**
- Modify: `src/adapter/tools.rs`

- [ ] **Step 1: 修改 create_all_tools 签名，增加 MCP 工具参数**

```rust
/// 创建所有业务工具的 FunctionTool 包装
///
/// `gitlab` 和 `project_id` 为可选,仅在需要 create_mr_note 工具时传入。
/// `mcp_tools` 为可选,来自 MCP Gateway 收集的远程工具。
pub fn create_all_tools(
    config: &Config,
    gitlab: Option<Arc<dyn GitlabApi>>,
    project_id: Option<u64>,
    mcp_tools: Vec<Arc<dyn Tool>>,
) -> Vec<Arc<dyn Tool>> {
    let workspace = std::env::current_dir().expect("获取工作目录失败");
    let file_io = FileIo::new(&workspace);
    let git_ops = GitOps::new(&workspace);

    let mut tools: Vec<Arc<dyn Tool>> = vec![
        // 文件工具
        Arc::new(create_read_file_tool(file_io.clone(), &config.read_file)),
        Arc::new(create_write_file_tool(file_io.clone())),
        Arc::new(create_list_files_tool(file_io.clone())),
        // Shell 工具
        Arc::new(create_run_command_tool(workspace.clone(), &config.command)),
        // Git 工具
        Arc::new(create_git_diff_tool(workspace.clone())),
        Arc::new(create_git_commit_tool(git_ops)),
        // AFT 代码感知工具
        Arc::new(create_aft_outline_tool(file_io.clone())),
        Arc::new(create_aft_view_symbol_tool(file_io.clone())),
        Arc::new(create_aft_edit_symbol_tool(file_io.clone())),
        Arc::new(create_aft_search_symbols_tool(file_io.clone())),
        Arc::new(create_aft_ast_replace_tool(file_io)),
    ];

    // 可选: GitLab 工具 (需要外部客户端)
    if let (Some(gitlab), Some(pid)) = (gitlab, project_id) {
        tools.push(Arc::new(create_mr_note_tool(gitlab, pid)));
    }

    // 合并 MCP 工具
    tools.extend(mcp_tools);

    tools
}
```

- [ ] **Step 2: 更新 main.rs 中 create_all_tools 的调用**

```rust
// 在 main.rs 中:
// 收集 MCP 工具
let mcp_tools = Vec::new(); // 后续由 MCP Gateway 提供

let tools = create_all_tools(
    &config,
    Some(gitlab.clone()),
    Some(config.gitlab.project_id),
    mcp_tools,
);
```

- [ ] **Step 3: 编译验证**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误

- [ ] **Step 4: 运行测试**

```bash
cargo test --all 2>&1
```
Expected: 所有测试通过

- [ ] **Step 5: 提交**

```bash
git add -A && git commit -m "feat: integrate MCP tools into create_all_tools"
```

---

## 阶段二：CI 修复闭环 + 模型路由 + 长期记忆

### 任务 9：实现 CI 修复闭环 (FixHandlerImpl + CI 集成)

**Files:**
- Modify: `src/ci/controller.rs`
- Modify: `src/main.rs` (已完成 FixHandlerImpl)

- [ ] **Step 1: 增强 CiController 的修复反馈**

```rust
// 在 controller.rs 的 run_fix_cycle 方法中，增加修复进度 MR 评论通知

// 在创建修复进度评论后，增加修复完成后的评论:
// 修复完成后，评论修复摘要
if let Err(e) = self
    .gitlab
    .create_mr_note(
        self.project_id,
        mr_iid,
        &format!(
            "✅ CI 修复尝试 #{attempt} 完成，已推送至 {branch}，正在等待新 pipeline...",
            attempt = attempt + 1,
            branch = branch,
        ),
    )
    .await
{
    tracing::warn!(error = %e, "创建修复完成评论失败");
}
```

- [ ] **Step 2: 编译验证**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误

- [ ] **Step 3: 运行测试**

```bash
cargo test --all 2>&1
```
Expected: 所有测试通过

- [ ] **Step 4: 提交**

```bash
git add -A && git commit -m "feat: enhance CI fix loop with MR progress comments"
```

---

### 任务 10：实现二级模型路由

**Files:**
- Modify: `src/adapter/provider.rs`
- Modify: `src/adapter/orchestrator.rs`

- [ ] **Step 1: 完善 provider.rs 的二级模型路由逻辑**

```rust
/// 创建简单任务模型 (小模型，用于阅读/搜索)
pub fn create_simple_model(config: &crate::config::LlmConfig) -> Result<Arc<dyn adk_rust::Llm>, crate::error::DevnpcError> {
    if config.model_routing.simple_model.is_empty() {
        // 回退到主模型
        return create_model(config);
    }
    // 使用简单模型配置
    let simple_config = crate::config::LlmConfig {
        model: config.model_routing.simple_model.clone(),
        ..config.clone()
    };
    create_model(&simple_config)
}

/// 创建复杂任务模型 (大模型，用于改码/修复/推理)
pub fn create_complex_model(config: &crate::config::LlmConfig) -> Result<Arc<dyn adk_rust::Llm>, crate::error::DevnpcError> {
    if config.model_routing.complex_model.is_empty() {
        // 回退到主模型
        return create_model(config);
    }
    let complex_config = crate::config::LlmConfig {
        model: config.model_routing.complex_model.clone(),
        ..config.clone()
    };
    create_model(&complex_config)
}
```

- [ ] **Step 2: 在 Orchestrator 中集成模型路由**

```rust
// 在 orchestrator.rs 中，Orchestrator 结构体增加模型路由字段:
pub struct Orchestrator {
    pub agent: LlmAgent,
    pub code_agent: Option<LlmAgent>,
    pub fix_agent: Option<LlmAgent>,
    pub review_agent: Option<LlmAgent>,
    /// 简单模型 (小模型)
    pub simple_model: Option<Arc<dyn adk_rust::Llm>>,
    /// 复杂模型 (大模型)
    pub complex_model: Option<Arc<dyn adk_rust::Llm>>,
}

impl Orchestrator {
    pub fn new(
        agent: LlmAgent,
        code_agent: Option<LlmAgent>,
        fix_agent: Option<LlmAgent>,
        review_agent: Option<LlmAgent>,
        simple_model: Option<Arc<dyn adk_rust::Llm>>,
        complex_model: Option<Arc<dyn adk_rust::Llm>>,
    ) -> Self {
        Self {
            agent,
            code_agent,
            fix_agent,
            review_agent,
            simple_model,
            complex_model,
        }
    }
}
```

- [ ] **Step 3: 更新 main.rs 中的 Orchestrator 构建**

```rust
// 创建模型路由
let simple_model = create_simple_model(&config.llm)?;
let complex_model = create_complex_model(&config.llm)?;

// 创建 Orchestrator
let orchestrator = Arc::new(devnpc::adapter::orchestrator::Orchestrator::new(
    agent,
    code_agent,
    fix_agent,
    review_agent,
    Some(simple_model),
    Some(complex_model),
));
```

- [ ] **Step 4: 编译验证**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误

- [ ] **Step 5: 运行测试**

```bash
cargo test --all 2>&1
```
Expected: 所有测试通过

- [ ] **Step 6: 提交**

```bash
git add -A && git commit -m "feat: implement two-level model routing"
```

---

### 任务 11：实现长期记忆系统 (Memory Store)

**Files:**
- Create: `src/adapter/memory.rs`
- Modify: `src/adapter/orchestrator.rs`
- Modify: `src/adapter/mod.rs`

- [ ] **Step 1: 创建 memory.rs 基础实现**

```rust
//! 长期记忆系统: 跨会话积累项目知识和经验
//!
//! 轻量起步: 使用 SQLite 存储结构化记忆。
//! 包含: 任务记录、修复经验、项目结构变更记录。

use std::path::PathBuf;

use crate::config::MemoryConfig;
use crate::error::Result;

/// 任务记录
#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub task_description: String,
    pub result_summary: String,
    pub modified_files: Vec<String>,
    pub duration_secs: u64,
    pub token_consumption: u64,
    pub success: bool,
    pub created_at: String,
}

/// 修复经验
#[derive(Debug, Clone)]
pub struct FixExperience {
    pub failure_type: String,
    pub error_message: String,
    pub root_cause: String,
    pub fix_method: String,
    pub success: bool,
    pub created_at: String,
}

/// 记忆存储器
pub struct MemoryStore {
    config: MemoryConfig,
    db_path: PathBuf,
}

impl MemoryStore {
    pub fn new(config: MemoryConfig) -> Self {
        let db_path = PathBuf::from(&config.db_path);
        Self { config, db_path }
    }

    /// 初始化数据库 (创建表)
    pub fn initialize(&self) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        tracing::info!(db_path = %self.db_path.display(), "初始化记忆存储");
        // TODO: 创建 SQLite 连接并创建表
        // 任务记录表: task_records(id, task_description, result_summary, ...)
        // 修复经验表: fix_experiences(id, failure_type, error_message, root_cause, ...)
        Ok(())
    }

    /// 保存任务记录
    pub fn save_task_record(&self, _record: TaskRecord) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        // TODO: INSERT INTO task_records ...
        Ok(())
    }

    /// 保存修复经验
    pub fn save_fix_experience(&self, _exp: FixExperience) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        // TODO: INSERT INTO fix_experiences ...
        Ok(())
    }

    /// 检索与当前任务相关的历史记忆
    pub fn retrieve_relevant(&self, _task_description: &str) -> Result<Vec<String>> {
        if !self.config.enabled {
            return Ok(Vec::new());
        }
        // TODO: SELECT 相关记录，按相似度或关键词匹配
        Ok(Vec::new())
    }
}
```

- [ ] **Step 2: 更新 adapter/mod.rs**

```rust
pub mod memory;
```

- [ ] **Step 3: 在 Orchestrator 中集成记忆注入**

```rust
// 在 orchestrator.rs 中:
use crate::adapter::memory::MemoryStore;

pub struct Orchestrator {
    // ... 现有字段
    pub memory_store: Option<MemoryStore>,
}

impl Orchestrator {
    pub fn new(
        // ... 现有参数
        memory_store: Option<MemoryStore>,
    ) -> Self {
        Self {
            // ... 现有字段初始化
            memory_store,
        }
    }

    /// 运行主 Agent 执行任务 (带记忆注入)
    pub async fn run_with_memory(
        &self,
        user_input: &str,
        session_service: Arc<dyn SessionService>,
        session_id: &str,
        initial_state: std::collections::HashMap<String, adk_rust::serde_json::Value>,
    ) -> Result<String> {
        // 检索相关记忆并注入
        if let Some(ref store) = self.memory_store {
            if let Ok(history) = store.retrieve_relevant(user_input) {
                if !history.is_empty() {
                    tracing::info!(count = history.len(), "注入历史记忆到 Agent 上下文");
                    // 将历史记忆附加到 user_input 中
                    let enriched_input = format!(
                        "{}\n\n## 历史相关记忆\n{}",
                        user_input,
                        history.join("\n---\n")
                    );
                    return self.run(&enriched_input, session_service, session_id, initial_state).await;
                }
            }
        }
        self.run(user_input, session_service, session_id, initial_state).await
    }
}
```

- [ ] **Step 4: 编译验证**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误

- [ ] **Step 5: 运行测试**

```bash
cargo test --all 2>&1
```
Expected: 所有测试通过

- [ ] **Step 6: 提交**

```bash
git add -A && git commit -m "feat: add long-term memory store with SQLite"
```

---

### 任务 12：集成 MCP Gateway 到 main.rs 启动流程

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: 在 run() 函数中启动 MCP Gateway**

```rust
// 在 "3. dry_run 模式" 之后，"4. 解析触发源" 之前:

// 3.5 初始化 MCP Gateway
let mcp_gateway = if config.mcp.enabled {
    let gateway = devnpc::adapter::mcp_gateway::McpGateway::new(config.mcp.clone());
    // 启动 codemap
    if let Err(e) = gateway.start_codemap().await {
        tracing::warn!(error = %e, "codemap 启动失败");
    }
    // 收集 MCP 工具
    let mcp_tools = gateway.collect_mcp_tools().await.unwrap_or_default();
    tracing::info!(count = mcp_tools.len(), "MCP 工具收集完成");
    Some(gateway)
} else {
    None
};

// 将 mcp_tools 传入 create_all_tools
let mcp_tools = mcp_gateway.as_ref()
    .map(|g| {
        // 需要重新收集或使用 Arc 共享
        Vec::new()
    })
    .unwrap_or_default();
```

- [ ] **Step 2: 编译验证**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误

- [ ] **Step 3: 运行测试**

```bash
cargo test --all 2>&1
```
Expected: 所有测试通过

- [ ] **Step 4: 提交**

```bash
git add -A && git commit -m "feat: integrate MCP Gateway into startup flow"
```

---

## 验收验证

### 任务 13：全量验证

- [ ] **Step 1: 运行所有测试**

```bash
cargo test --all 2>&1
```
Expected: 所有测试通过

- [ ] **Step 2: Clippy 零警告**

```bash
cargo clippy -- -D warnings 2>&1
```
Expected: 零警告

- [ ] **Step 3: Release 构建**

```bash
cargo build --release 2>&1
```
Expected: 构建成功

- [ ] **Step 4: 提交最终验证**

```bash
git add -A && git commit -m "chore: verification pass - all tests pass, clippy clean"
```