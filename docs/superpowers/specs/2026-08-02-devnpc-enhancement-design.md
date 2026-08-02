# devnpc 增强方案设计

> 日期: 2026-08-02
> 项目: https://github.com/duantianjun/devnpc
> 基于: docs/devnpc-技术分析报告.md

---

## 一、总体方案

采用**方案 B：先重构后增强**。先重构为多 Agent 架构，再逐步添加各子系统。

### 执行顺序

```
S3 (MCP Gateway) → S1 (代码知识图谱) → S2 (CI 修复闭环) → M1 (模型路由) → M2 (多 Agent 协作) → M3 (长期记忆)
```

### 整体架构

```
┌──────────────────────────────────────────────────────────────┐
│                        devnpc 增强架构                        │
├──────────────────────────────────────────────────────────────┤
│  Orchestrator Agent (M2) ← 任务拆解 + 分发 + 汇总            │
│  ├── SubAgent: Code Agent (M2)  ← 代码修改专用               │
│  ├── SubAgent: Fix Agent (S2)   ← CI 修复专用                │
│  └── SubAgent: Review Agent (M2)← 代码审查专用               │
├──────────────────────────────────────────────────────────────┤
│  Model Router (M1) ← 简单/复杂任务自动路由                    │
│  ├── 小模型: DeepSeek Chat / GPT-4o-mini  ← 阅读/搜索/简单   │
│  └── 大模型: DeepSeek Reasoner / GPT-4o  ← 改码/修复/推理    │
├──────────────────────────────────────────────────────────────┤
│  MCP Gateway (S3) ← 统一 MCP 协议入口                        │
│  ├── MCP: codemap (S1)  ← 代码知识图谱                       │
│  ├── MCP: GitLab API    ← 工单/CI 数据                       │
│  └── MCP: Memory Store (M3) ← 长期记忆                       │
├──────────────────────────────────────────────────────────────┤
│  Long-Term Memory (M3) ← SQLite/向量数据库                    │
└──────────────────────────────────────────────────────────────┘
```

---

## 二、设计一：多 Agent 架构重构（M2 前置基础）

### 目标

将当前单体 Agent 重构为 Orchestrator + 多个子 Agent 的层级式编排架构。

### 架构

```
用户请求
    │
    ▼
Orchestrator Agent ─── 任务拆解 → 子任务分配 → 结果汇总
    │
    ├── Code Agent     ← 代码读写、AST 操作、编译验证
    ├── Fix Agent      ← CI 日志分析、根因定位、修复代码
    └── Review Agent   ← 代码审查、SOP 合规检查
```

### 核心设计

- **Orchestrator Agent**：使用 adk-rust 的 `LlmAgent` + `Runner`，将子 Agent 调用封装为 `FunctionTool`。负责任务意图识别、拆解为子任务、分配给对应子 Agent、协调执行顺序、汇总结果
- **子 Agent**：每个子 Agent 通过 `LlmAgentBuilder` 构建，拥有专属的 System Prompt 和工具集
- **通信机制**：子 Agent 不直接相互调用，通过 Orchestrator 传递中间结果，保持解耦
- **状态共享**：所有子 Agent 共享同一个 Session，通过 Session State 传递上下文

### 工具分配

| Agent | 工具 |
|-------|------|
| **Code Agent** | read_file, write_file, list_files, git_diff, git_commit, aft_outline, aft_view_symbol, aft_edit_symbol, aft_search_symbols, aft_ast_replace, run_command |
| **Fix Agent** | read_file, write_file, aft_outline, aft_view_symbol, aft_edit_symbol, run_command, create_mr_note |
| **Review Agent** | read_file, list_files, aft_outline, aft_view_symbol, aft_search_symbols, create_mr_note |

### 实现方式

- `LlmAgentBuilder` 构建每个子 Agent
- `Runner` 执行单个 Agent
- 当前 `main.rs` 中的 `run()` 函数改为 Orchestrator 逻辑：先调 Code Agent 执行 → 如需要 CI 修复则调 Fix Agent → 最后调 Review Agent

### 编排流程

```
用户任务 "修复登录模块的 token 刷新 bug"
    │
    ▼
Orchestrator 分析:
  ├── 子任务 1: [Code Agent] 阅读登录模块源码，定位 bug
  ├── 子任务 2: [Code Agent] 修改代码修复 bug
  ├── 子任务 3: [Code Agent] 运行 cargo build 验证编译
  ├── 子任务 4: [Code Agent] git commit + push
  ├── 子任务 5: [Fix Agent]  等待 CI，失败则自动修复
  └── 子任务 6: [Review Agent] 审查最终代码
    │
    ▼
Orchestrator 按依赖顺序执行 → 汇总结果 → 生成报告
```

### 文件变更

- `src/adapter/agents.rs` — 新增：子 Agent 构建逻辑
- `src/adapter/orchestrator.rs` — 新增：Orchestrator 编排逻辑
- `src/adapter/mod.rs` — 修改：导出新模块
- `src/main.rs` — 修改：精简 `run()` 函数，委托给 Orchestrator

---

## 三、设计二：MCP Gateway（S3）

### 目标

利用 adk-rust 自带的 `mcp` feature，建立统一的 MCP 协议入口，使 devnpc 能对接外部 MCP 服务器。

### 架构

```
devnpc Agent
    │
    ▼
MCP Gateway ─── 管理 MCP 客户端连接
    │
    ├── MCP: codemap (Rust)   ← 代码知识图谱（S1）
    ├── MCP: GitLab API       ← 工单/CI/仓库数据
    └── MCP: Memory Store     ← 长期记忆（M3）
```

### 核心设计

- **启用 adk-rust 的 `mcp` feature**：在 `Cargo.toml` 中已包含 `"mcp"` feature，直接启用
- **MCP 客户端管理器**：`McpGateway` 负责管理多个 MCP 服务器连接（stdio 或 streamable HTTP），支持动态注册/注销
- **工具桥接**：MCP 服务器暴露的工具自动桥接为 adk-rust 的 `FunctionTool`，Agent 无需感知底层是本地工具还是 MCP 远程工具
- **配置化**：MCP 服务器列表通过配置文件（`.devnpc.md` 或环境变量）声明

### 配置方式

```yaml
mcp_servers:
  - name: codemap
    transport: stdio
    command: codemap
    args: ["serve"]
  - name: gitlab
    transport: http
    url: http://localhost:9090/mcp
```

### 文件变更

- `src/adapter/mcp_gateway.rs` — 新增：MCP Gateway 实现
- `src/adapter/mod.rs` — 修改：导出新模块
- `src/config/mod.rs` — 修改：新增 `McpConfig` 配置结构
- `src/config/env.rs` — 修改：支持 MCP 服务器配置的环境变量注入
- `src/adapter/tools.rs` — 修改：`create_all_tools` 增加 MCP 工具收集逻辑

---

## 四、设计三：代码知识图谱（S1）

### 目标

通过集成 codemap (Rust MCP 服务器)，将当前每次执行都重新解析文件的 tree-sitter AFT 工具，升级为持久化知识图谱查询，token 消耗降低 90%+。

### 架构

```
MCP Gateway
    │
    ▼
codemap MCP Server ─── 单一 Rust 二进制，零运行时依赖
    │
    ├── tree-sitter 解析 → 持久化 SQLite 知识图谱
    ├── 增量更新（文件监听 + SHA-256 哈希）
    └── 17 个 MCP 工具（搜索/调用链/影响分析等）
```

### 集成方式

- codemap 作为独立子进程，通过 MCP stdio 协议与 devnpc 通信
- 在 devnpc 启动时自动启动 codemap 进程
- 知识图谱数据存储在项目根目录 `.codemap/` 下，跨会话持久化
- 文件变更时自动增量更新

### 工具映射

| 现有 AFT 工具 | codemap 替代 | 说明 |
|---------------|-------------|------|
| `aft_outline` | `codemap-file` | 列出文件所有符号，更快更省 token |
| `aft_view_symbol` | `codemap-definition` | 查看符号定义源码 |
| `aft_search_symbols` | `codemap-search` | 全局符号名搜索 |
| `aft_edit_symbol` | 保留现有 | 符号替换仍需本地写入 |
| `aft_ast_replace` | 保留现有 | AST 正则替换仍需本地写入 |
| 无 | `codemap-callers` | **新增**：谁调用了此函数 |
| 无 | `codemap-impact` | **新增**：修改此函数影响分析 |
| 无 | `codemap-unused` | **新增**：死代码检测 |

### 文件变更

- `src/adapter/mcp_gateway.rs` — 修改：添加 codemap 服务器启动逻辑
- `src/adapter/tools.rs` — 修改：保留 AFT 编辑工具，新增 MCP 查询工具
- `src/config/mod.rs` — 修改：codemap 路径配置项

---

## 五、设计四：CI 修复闭环（S2）

### 目标

将当前 `NoopFixHandler` 占位替换为真正的修复 Agent，实现"日志解析 → 根因定位 → 代码修复 → 重试验证"全闭环。

### 架构

```
CI 控制器检测到 Pipeline 失败
    │
    ▼
日志解析器 (log_parser) ─── 提取错误信息、失败命令、堆栈
    │
    ▼
Fix Agent ─── 全量修复模式，可修改任何代码文件
    │
    ├── 1. 分析日志，定位根因
    ├── 2. 读取相关代码上下文
    ├── 3. 修改代码修复问题
    ├── 4. git commit + git push
    └── 5. 触发重新 CI
    │
    ▼
CI 控制器继续轮询，重复直到成功或达到 max_ci_retries
```

### 核心实现

```rust
struct FixHandlerImpl {
    orchestrator: Arc<Orchestrator>,
}

#[async_trait]
impl FixHandler for FixHandlerImpl {
    async fn run_fix(&self, failures: &[ParsedFailure], instruction: &str) -> Result<String> {
        let context = build_fix_context(failures, instruction);
        let result = self.orchestrator.run_fix_agent(context).await?;
        Ok(result)
    }
}
```

### 修复流程

1. `CiController` 检测到 Pipeline 失败，获取 Job 日志
2. `log_parser` 解析日志，提取 `ParsedFailure`（错误类型、文件位置、行号、错误消息）
3. 调用 `FixHandlerImpl::run_fix`，传入所有失败信息和原始任务描述
4. Fix Agent 启动：读取日志上下文 → 读取相关源码 → 定位根因 → 生成修复补丁 → 验证语法 → 写入文件
5. 修复完成后 `git commit` + `git push`
6. `CiController` 继续轮询 Pipeline，重复直到成功或达到最大重试次数

### 文件变更

- `src/main.rs` — 修改：`NoopFixHandler` → `FixHandlerImpl`
- `src/ci/controller.rs` — 修改：`CiController` 增加修复回路集成
- `src/adapter/orchestrator.rs` — 修改：增加 `run_fix_agent` 方法
- `src/adapter/agents.rs` — 修改：Fix Agent 构建

---

## 六、设计五：模型路由（M1）

### 目标

实现二级模型路由，简单任务走小模型（节省成本），复杂任务走大模型（保证质量）。

### 架构

```
Agent 请求
    │
    ▼
Model Router
    │
    ├── 任务分类器 ← 分析任务描述，判定复杂度
    │   ├── 简单: 文件读取、目录列表、git diff、符号搜索
    │   └── 复杂: 代码修改、CI 修复、多步推理、代码审查
    │
    ├── 小模型 (DeepSeek Chat / GPT-4o-mini)
    │   └── 用于 Code Agent 中的阅读/搜索操作
    │
    └── 大模型 (DeepSeek Reasoner / GPT-4o)
        └── 用于 Code Agent 的修改操作 + Fix Agent + Review Agent
```

### 路由策略

```
Agent 发起 LLM 调用
    │
    ▼
Model Router 检查本次调用是否涉及"写"操作
    ├── 是（write_file / aft_edit_symbol / git_commit）→ 大模型
    └── 否（read_file / list_files / aft_outline / aft_search）→ 小模型
```

### 文件变更

- `src/adapter/provider.rs` — 修改：支持二级模型创建
- `src/adapter/orchestrator.rs` — 修改：Orchestrator 根据任务类型选择模型
- `src/adapter/agents.rs` — 修改：Code Agent 配置双模型

---

## 七、设计六：长期记忆系统（M3）

### 目标

跨会话积累项目知识和经验，减少重复学习，提高 Agent 执行效率。

### 架构

```
Agent 执行完成
    │
    ▼
Memory Collector
    ├── 提取本次执行的关键信息
    │   ├── 任务描述 + 执行结果
    │   ├── 修改了哪些文件、修改了什么
    │   ├── CI 失败原因 + 修复方法
    │   └── 项目结构变化（新文件/新模块）
    │
    ▼
Memory Store (MCP 服务)
    ├── SQLite: 结构化记忆（任务记录、修复经验）
    └── 向量存储: 语义记忆（代码片段、决策理由）
    │
    ▼
下次执行时:
Memory Retriever → 检索相关历史记忆 → 注入 Agent 上下文
```

### 存储内容

| 类型 | 内容 | 存储方式 |
|------|------|----------|
| 任务记录 | 任务描述、执行结果、耗时、token 消耗 | SQLite 表 |
| 修复经验 | CI 失败类型、根因、修复方法、是否成功 | SQLite 表 |
| 项目结构 | 新增模块、新增 API、文件变更记录 | SQLite 表 |
| 关键决策 | 为什么选择某种实现方式 | 向量存储 |

### 实现方式

- **轻量起步**：先用 SQLite 存储结构化记忆，通过 MCP 服务暴露查询接口
- **记忆注入**：Orchestrator 启动时，从 Memory Store 检索与当前任务相关的历史记录，注入 System Prompt
- **渐进增强**：后续可升级为向量数据库（如 `qdrant` 或 `pgvector`）支持语义搜索

### 文件变更

- `src/adapter/memory.rs` — 新增：Memory Store MCP 服务
- `src/adapter/orchestrator.rs` — 修改：Orchestrator 集成记忆注入
- `src/adapter/mcp_gateway.rs` — 修改：注册 Memory Store MCP 服务
- `src/config/mod.rs` — 修改：Memory Store 配置项

---

## 八、文件变更汇总

### 新增文件

| 文件 | 说明 |
|------|------|
| `src/adapter/agents.rs` | 子 Agent 构建（Code/Fix/Review Agent） |
| `src/adapter/orchestrator.rs` | Orchestrator 编排逻辑 |
| `src/adapter/mcp_gateway.rs` | MCP Gateway 管理 |
| `src/adapter/memory.rs` | 长期记忆 MCP 服务 |

### 修改文件

| 文件 | 修改内容 |
|------|----------|
| `src/main.rs` | 集成 Orchestrator，替换 NoopFixHandler |
| `src/adapter/mod.rs` | 导出新模块 |
| `src/adapter/tools.rs` | 集成 MCP 工具 + 知识图谱工具 |
| `src/adapter/provider.rs` | 支持二级模型创建 |
| `src/ci/controller.rs` | 集成 FixHandlerImpl |
| `src/config/mod.rs` | 新增 McpConfig、MemoryConfig |

---

## 九、执行顺序与依赖关系

```
S3 (MCP Gateway) ── 无依赖
    │
    ▼
S1 (代码知识图谱) ── 依赖 S3 (通过 MCP 接入 codemap)
    │
    ▼
S2 (CI 修复闭环) ── 依赖 M2 (Fix Agent 作为子 Agent)
    │
    ▼
M1 (模型路由) ──── 无依赖，可独立实现
    │
    ▼
M2 (多 Agent 协作) ─ 依赖 M1 (路由决策需要模型路由)
    │
    ▼
M3 (长期记忆) ──── 依赖 S3 (通过 MCP 暴露记忆服务)
```

实际分两阶段实现：

**阶段一 (先重构)**：M2 基础架构 + S3 + S1
- 先搭建多 Agent 骨架（Orchestrator + 子 Agent 基础结构）
- 同时建立 MCP Gateway
- 接入 codemap 知识图谱
- 三者可并行开发

**阶段二 (后增强)**：S2 + M1 + M3
- 实现真正的 CI 修复闭环
- 实现模型路由
- 实现长期记忆系统

---

## 十、验收标准

1. `cargo test --all` 全通过
2. `cargo clippy -- -D warnings` 零警告
3. `cargo build --release` 成功
4. Orchestrator 能正确拆解任务并调度子 Agent
5. Fix Agent 能解析 CI 日志并修复代码
6. codemap 知识图谱查询正常返回结果
7. 模型路由按任务类型自动选择模型
8. 长期记忆跨会话持久化