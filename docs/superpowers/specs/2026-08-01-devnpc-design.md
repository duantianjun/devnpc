# devnpc 设计文档

- **状态**: 已评审通过,待生成实施计划
- **创建日期**: 2026-08-01
- **作者**: devnpc 设计协作
- **对标产品**: 腾讯 CodeBuddy NPC

## 摘要

devnpc 是一个基于 GitLab 的企业级研发流程 AI 智能体,用 Rust 实现,部署为 GitLab CI/CD 作业。开发者通过 `@devnpc` 派发任务,NPC 自主读取研发记忆(Git 仓库 / Issue / PR / CI)、规划方案、编写代码、提交 MR、执行 CI 并在失败时自动修复,直至交付可验收成果。设计对标腾讯 CodeBuddy NPC 的五项核心能力:全流程自主交付、研发记忆驱动、多 NPC 协同组队、智能冲突与 CI 修复、企业灵活配置。

## 核心决策汇总

| # | 决策项 | 选择 |
|---|---|---|
| Q1 | 范围分解 | 单 spec 全量设计,分阶段实施 |
| Q2 | GitLab 部署形态 | 自建 GitLab (16.x/17.x) |
| Q3 | 主测 LLM | DeepSeek (OpenAI 兼容协议) |
| Q4 | GitLab 认证 | 项目访问令牌 (PAT, scope=api) |
| Q5 | 持久化记忆 | 仅 Git 作为记忆 (AI Native Git) |
| Q6 | 项目指令文件 | `.devnpc.md` (front matter + Markdown) |
| Q7 | NPC 间通信 | GitLab Issue/MR 评论总线 (可审计优先) |
| Q8 | 模型路由 | MVP 单模型,路由放 P8 |
| D5 | CI 修复重试上限 | 3 次 |
| D10 | 分支命名 | `npc/<issue-id>-<slug>` |
| A1 | Agent 架构 | 方案 C: ReAct + SOP 双层 |
| A2 | LLM 抽象层 | rig-core |
| A3 | 代码工具层 | agent-file-tools (AFT, tree-sitter) |
| A4 | 运维页面 | 路径 C: 静态报告页 (Artifact/GitLab Pages) |
| A5 | HTML 生成 | askama (编译期模板) |
| A6 | Skill 抽象 | MVP 不实现, P6 再评估 (YAGNI) |

---

## 1. 项目定位与目标

### 1.1 项目定位

devnpc 是一个基于 GitLab 的企业级研发流程 AI 智能体,用 Rust 实现,部署为 GitLab CI/CD 作业。开发者通过 `@devnpc` 派发任务,NPC 自主读取研发记忆(Git 仓库 / Issue / PR / CI)、规划方案、编写代码、提交 MR、执行 CI 并在失败时自动修复,直至交付可验收成果。

### 1.2 对标 CodeBuddy NPC 的能力映射

| CodeBuddy 能力 | devnpc 实现方式 | 实施阶段 |
|---|---|---|
| 全流程自主交付 | ReAct Agent 循环 + CI 闭环控制器 | P3-P5 |
| 研发记忆驱动 | Git 仓库上下文聚合器 (AI Native Git) | P2 |
| 多 NPC 协同组队 | Team 编排器 + GitLab 评论总线 | P7 |
| 智能冲突与 CI 修复 | CI 闭环控制器 + 日志解析器 | P4 |
| 企业灵活配置 | `.devnpc.md` + Role/SOP YAML | P6 |

### 1.3 成功标准

- **MVP (P0-P5)**: 在自建 GitLab 项目中,于 Issue 评论 `@devnpc 修复 #42 的登录 bug`,NPC 能自主完成:读取 Issue → clone 仓库 → 改码 → 提交 MR → CI 通过 → 评论验收摘要。全程无人工干预。
- **完整版 (P0-P9)**: 支持 PM/DEV/TEST 多 NPC 协同完成复杂功能迭代,可加载企业自定义 Role/SOP,按任务复杂度路由模型。

### 1.4 非目标 (YAGNI)

- 不做本地 IDE 插件 (CodeBuddy 有三端,devnpc 聚焦 CI 作业形态)
- 不做常驻 Web 服务 (仅静态报告页)
- 不做自建容器沙箱 (复用 GitLab Runner 沙箱)
- 不做外部持久化存储 (记忆全在 Git)
- 模型路由 P8 之前不做
- Skill 抽象 MVP 不实现,P6 再评估

---

## 2. 整体架构与组件划分

### 2.1 三层架构

```
┌──────────────────────────────────────────────────────────┐
│ 第 1 层: agent/ (自建 ReAct 循环 + SOP 双层)              │
│  · loop.rs: LLM ↔ Tool 循环 + SOP 偏离检测 + 迭代上限   │
│  · prompt.rs: 系统提示词 + SOP 步骤注入                  │
│  · llm_client.rs: rig provider 封装                      │
├──────────────────────────────────────────────────────────┤
│ 第 2 层: tools/ (Agent 工具集,唯一副作用出口)            │
│  ├─ file_io.rs   ← 基于 AFT (tree-sitter 符号级读改)     │
│  │   · view_symbol / edit_symbol / ast_replace           │
│  │   · outline / search_symbols                          │
│  ├─ shell.rs     (cargo/test/lint,沙箱内)                │
│  ├─ git_tool.rs  (clone/branch/commit/push)              │
│  └─ gitlab_tool.rs (Issue/MR/Pipeline/Notes)             │
├──────────────────────────────────────────────────────────┤
│ 第 3 层: 基础设施                                         │
│  · rig-core (LLM 抽象,切 DeepSeek/GLM/OpenAI)            │
│  · agent-file-tools (代码感知,AFT)                       │
│  · reqwest (GitLab REST v4)                              │
│  · tokio (异步运行时)                                    │
└──────────────────────────────────────────────────────────┘
```

### 2.2 部署架构

```
┌─────────────────────────────────────────────────────────────┐
│              自建 GitLab (16.x/17.x)                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────────────────┐       │
│  │ Issue/   │  │ Runner + │  │ GitLab Pages (可选)  │       │
│  │ MR 评论  │  │ CI 沙箱  │  │  托管运维报告 HTML   │       │
│  └────┬─────┘  └────┬─────┘  └──────────────────────┘       │
│       │ @devnpc 触发 │ devnpc 二进制在此执行                 │
└───────┼──────────────┼──────────────────────────────────────┘
        │              │
        │     ┌────────┴─────────────────────────┐
        │     │  devnpc (单 Rust 二进制)          │
        │     │  trigger → team → npc → agent    │
        │     │            → ci → report         │
        │     └──────────────┬──────────────────┘
        │                    │
        ▼                    ▼ HTTPS
   ┌─────────┐         ┌──────────────┐
   │GitLab   │         │ LLM (DeepSeek│
   │API v4   │         │ OpenAI兼容)  │
   └─────────┘         └──────────────┘
```

### 2.3 组件清单与职责

每个组件单一职责、清晰接口、可独立测试。

| 组件 | 职责 | 依赖 | 接口要点 |
|---|---|---|---|
| `config/` | 加载 env + `.devnpc.md` + YAML 角色 | dotenvy, serde | `Config::load() -> Config` |
| `gitlab_api/` | REST API v4 客户端 (Issue/MR/Pipeline/Notes/Repo) | reqwest | `GitlabClient::new(url, token)` |
| `git/` | 调系统 git 命令 (clone/branch/commit/push) | std::process | `GitOps::clone(url, branch)` |
| `memory/` | 聚合研发记忆 (目录树/关键文件/Issue/PR/CI) | gitlab_api, git | `Context::build(issue_id) -> Context` |
| `agent/` | ReAct 循环 + LLM 客户端 + 提示词 | rig-core | `Agent::run(task, tools) -> Result` |
| `tools/` | Agent 可调用工具 (AFT+shell+git+gitlab) | agent-file-tools | `Tool::call(args) -> Result` |
| `ci/` | CI 闭环: 创建MR → 轮询 → 解析日志 → 触发修复 | gitlab_api, agent | `CiController::run(mr) -> Result` |
| `npc/` | 单 NPC 执行器 + Role/SOP 加载 | agent, memory, ci | `NpcRunner::execute(task)` |
| `team/` | 多 NPC 编排 (任务分解/并行/联调) | npc, gitlab_api(评论总线) | `Orchestrator::run(goal)` |
| `trigger/` | 解析 @devnpc 提及 + Issue/MR 上下文 | gitlab_api | `parse_trigger(event) -> Task` |
| `report/` | 记录轨迹 + 生成 HTML + 推送 Pages | tracing, askama, gitlab_api | `Reporter::finalize(run)` |
| `error.rs` | 统一错误类型 | thiserror | `DevnpcError` enum |

### 2.4 数据流 (单 NPC 主闭环)

```
1. GitLab CI 触发 → config::load()
2. trigger::parse(CI 变量) → Task
3. memory::context::build(issue_id)
   → gitlab_api 取 Issue/PR/CI 历史 + .devnpc.md
   → git 取目录树 + 关键文件
4. npc::runner::execute(Task, Context)
   a. git clone + branch(npc/<issue-id>-<slug>)
   b. agent loop: LLM ↔ tools(AFT 改码/shell 验证) 循环
   c. git commit + push
5. ci::controller::run()
   → create_mr → 轮询 pipeline
   → 失败: get_job_logs → log_parser → 喂回 agent 修复 → push
   → 循环最多 3 次
6. report::finalize(轨迹) → HTML → GitLab Pages/Artifact
7. gitlab_api::comment(mr, 验收摘要 + 报告链接)
```

### 2.5 关键设计原则

1. **单一二进制**: 所有功能编译进一个 `devnpc` 二进制,CI 作业直接调用,无运行时依赖
2. **沙箱内执行**: 所有 git/文件/shell 操作限制在 CI Runner 工作目录,不访问 Runner 外
3. **Git 即记忆**: 跨任务状态全靠 GitLab API 重建,进程无状态
4. **工具是 Agent 的手脚**: `tools/` 是 Agent 唯一副作用出口,便于审计与权限收口
5. **报告与执行解耦**: `report/` 通过 `tracing` 订阅执行事件,不侵入业务逻辑

---

## 3. Agent 核心与工具系统

### 3.1 ReAct 循环结构 (方案 C: ReAct + SOP 双层)

```rust
// agent/loop.rs 核心伪代码
pub async fn run(
    agent: &Agent,
    task: &Task,
    context: &Context,
    tools: &ToolRegistry,
    sop: Option<&Sop>,
    cancel: CancellationToken,
) -> Result<RunResult> {
    let mut messages = build_initial_messages(task, context, sop);
    let mut trajectory = Trajectory::new();
    
    for iteration in 0..agent.max_iterations {  // 默认 20
        if cancel.is_cancelled() { return Err(Cancelled); }
        
        // 1. 调 LLM
        let response = agent.llm.complete(&messages, &tools.schemas()).await?;
        trajectory.record_llm_call(&response);
        
        // 2. 检查完成
        let tool_calls = response.extract_tool_calls();
        if tool_calls.is_empty() {
            return Ok(RunResult::Finished(response.text, trajectory));
        }
        
        // 3. SOP 偏离检测 (方案 C 核心,软约束默认)
        if let Some(sop) = sop {
            check_sop_deviation(&sop, &tool_calls, &trajectory)?;
        }
        
        // 4. 执行工具 (并行)
        let results = futures::future::join_all(
            tool_calls.iter().map(|tc| tools.call(tc, &cancel))
        ).await;
        
        // 5. 喂回结果
        messages.extend(response.into_messages());
        messages.extend(results.into_tool_messages());
        trajectory.record_tool_calls(&tool_calls, &results);
    }
    
    Ok(RunResult::MaxIterationsReached(trajectory))
}
```

关键设计:
- **迭代上限**: 默认 20,可配 (`DEVNPC_MAX_ITERATIONS`),防死循环
- **SOP 偏离检测**: 默认软约束 (只警告),`.devnpc.md` 可配 `sop_mode: strict` 切硬阻断
- **并行工具调用**: LLM 一次返回多个 tool_call 时并行执行
- **轨迹记录**: 每次 LLM 调用 + 工具调用都记入 trajectory,供 report 模块消费
- **SOP 步骤推断**: trajectory 历史 + LLM 显式声明当前步双重信号

### 3.2 提示词结构 (agent/prompt.rs)

```
[系统提示词]
  你是 {role.name},{role.description}
  遵循以下 SOP (可偏离但需说明理由):
  {sop.steps}
  
  项目规范 (来自 .devnpc.md):
  {project_guidelines}
  
  工作目录: {workspace}
  分支: {branch}
  禁止: 访问工作目录外文件 / 直接 push 到主分支
  
[研发记忆] (来自 memory 模块)
  仓库结构: {repo_tree}
  目标 Issue: {issue_content}
  相关 PR 历史: {related_prs}
  已知 CI 失败模式: {known_failures}
  
[任务]
  {task.description}
  验收标准: {task.acceptance_criteria}

[对话历史]
  ... (ReAct 循环累积)
```

### 3.3 工具集定义 (tools/)

13 个工具,rig 的 macro 自动生成 JSON schema。

| 工具 | 来源 | 参数 | 作用 |
|---|---|---|---|
| `view_symbol` | AFT | file, symbol_name | 只读目标符号 (省 Token) |
| `edit_symbol` | AFT | file, symbol_name, new_code | 按符号名改代码 |
| `ast_replace` | AFT | file, pattern, replacement | ast-grep 结构化替换 |
| `outline` | AFT | file | 返回文件大纲 |
| `search_symbols` | AFT | query, scope | 符号搜索 |
| `read_file` | 自建 | path | 全量读 (小文件兜底) |
| `write_file` | 自建 | path, content | 全量写 (新文件) |
| `list_files` | 自建 | dir | 列目录 |
| `git_diff` | 自建 | (无) | 查看当前改动 |
| `run_command` | 自建 | cmd, args, timeout | 跑 cargo test/build/lint |
| `git_commit` | 自建 | message | 提交当前改动 |
| `create_mr_note` | gitlab | mr_iid, body | 在 MR 评论 (CI 闭环用) |
| `finish` | 自建 | summary | 标记任务完成 |

工具安全约束:
- 所有文件操作限制在 workspace 内 (path traversal 检查)
- `run_command` 有白名单/黑名单 + 超时
- `git_commit` 只能提交到当前 npc 分支,不能切主分支

### 3.4 SOP 偏离检测 (方案 C 核心)

```rust
struct Sop {
    steps: Vec<SopStep>,
}

struct SopStep {
    name: String,
    expected_tools: Vec<String>,
    description: String,
}

fn check_sop_deviation(
    sop: &Sop,
    tool_calls: &[ToolCall],
    trajectory: &Trajectory,
) -> Result<DeviationReport> {
    let current_step = sop.estimate_current_step(trajectory);
    let unexpected: Vec<_> = tool_calls.iter()
        .filter(|tc| !current_step.expected_tools.contains(&tc.name))
        .collect();
    
    if unexpected.is_empty() {
        Ok(DeviationReport::None)
    } else {
        // 软约束: 只记录,在下轮提示 LLM
        // strict 模式: 返回错误阻断
        Ok(DeviationReport::Soft {
            step: current_step.name,
            unexpected_tools: unexpected,
        })
    }
}
```

示例 SOP (bug 修复):
```yaml
# npc-config/sops/bugfix.yml
name: bugfix
steps:
  - name: 复现
    expected_tools: [run_command, read_file, view_symbol]
  - name: 定位
    expected_tools: [search_symbols, view_symbol, outline]
  - name: 修复
    expected_tools: [edit_symbol, ast_replace, write_file]
  - name: 验证
    expected_tools: [run_command, git_commit, git_diff]
  - name: 完成
    expected_tools: [finish]
```

MVP 阶段 SOP 可为空 (等价纯 ReAct),P6 再填充。

---

## 4. 配置系统与项目指令

### 4.1 三层配置来源 (优先级从高到低)

1. 环境变量 / GitLab CI 变量 ← 最高优先级,运维覆盖
2. `.devnpc.md` (项目根目录) ← 项目级规范,随 Git 版本化
3. `npc-config/` (NPC 仓库内) ← 默认角色/SOP 定义

### 4.2 环境变量契约

```bash
# === 必需 ===
DEVNPC_API_KEY              # LLM API 密钥
DEVNPC_BASE_URL             # 模型服务地址 (OpenAI 兼容),如 https://api.deepseek.com/v1
DEVNPC_MODEL                # 默认模型名,如 deepseek-chat

# === GitLab (自建) ===
GITLAB_TOKEN                # 项目访问令牌 (scope=api),CI 内可缺省走 CI_JOB_TOKEN
# 以下 GitLab CI 自动注入,无需配置:
# CI_API_V4_URL / CI_PROJECT_ID / CI_PROJECT_URL
# CI_MERGE_REQUEST_IID / CI_COMMIT_SHA / CI_PIPELINE_ID

# === 可选 ===
DEVNPC_MAX_ITERATIONS=20    # Agent 循环上限
DEVNPC_MAX_CI_RETRIES=3     # CI 修复重试上限
DEVNPC_SOP_MODE=soft        # soft (默认) | strict
DEVNPC_MODEL_ROUTING=       # P8 模型路由 JSON,MVP 留空
DEVNPC_REPORT_TARGET=artifact  # artifact (默认) | pages | none
RUST_LOG=info               # 日志级别
```

### 4.3 `.devnpc.md` 格式 (项目指令文件)

采用 Markdown + YAML front matter,兼顾人读与机读。

```markdown
---
# 机器读: 硬约束
sop_mode: strict              # strict 时 SOP 偏离即阻断
forbidden_paths:              # 禁止 NPC 修改
  - ".gitlab-ci.yml"
  - "Cargo.lock"
  - "migrations/"
required_checks:              # 提交前必须通过的本地命令
  - "cargo fmt --check"
  - "cargo clippy -D warnings"
  - "cargo test"
branch_prefix: "npc"          # 分支前缀,默认 npc
max_ci_retries: 3             # 覆盖环境变量
---

# 项目规范 (人读 + LLM 读)

## 技术栈
- Rust 2024 + Tokio
- 数据库: PostgreSQL 16 + sqlx

## 编码约定
- 所有公共函数必须有 doc comment
- 错误处理用 thiserror,禁止 unwrap/expect 在生产代码
- 测试与实现同文件,#[cfg(test)] mod tests

## 架构约束
- 不允许在 handler 层直接写 SQL,必须经 repository 层
- 新增依赖需在 PR 说明理由

## 常用命令
- 构建: `cargo build --release`
- 测试: `cargo test -- --test-threads=4`
- 检查: `make ci` (等价 fmt+clippy+test)
```

解析规则:
- front matter 由 `serde_yaml` 解析为 `ProjectConfig` struct
- 正文整体作为 system prompt 注入 (人读部分 LLM 也能读)

### 4.4 NPC 角色定义 (npc-config/roles/*.yml)

```yaml
# npc-config/roles/developer.yml
name: developer
description: 全栈开发 NPC,负责实现功能与修复 Bug
model:                        # 可覆盖全局模型
  provider: deepseek          # P8 路由时生效
  name: deepseek-chat
system_prompt: |
  你是资深全栈工程师。遵循项目规范,优先用最小改动解决问题。
  修改前先用 view_symbol 理解上下文,改完用 run_command 验证。
max_iterations: 25            # 覆盖默认 20
default_sop: bugfix           # 默认 SOP
tools:                        # 启用的工具子集
  - view_symbol
  - edit_symbol
  - ast_replace
  - run_command
  - git_commit
  - git_diff
  - finish
```

### 4.5 SOP 定义 (npc-config/sops/*.yml)

```yaml
# npc-config/sops/feature.yml
name: feature
description: 新功能开发流程
steps:
  - name: 设计
    expected_tools: [list_files, outline, view_symbol, search_symbols]
    hint: "先理解现有架构,找出扩展点"
  - name: 实现
    expected_tools: [edit_symbol, ast_replace, write_file, view_symbol]
    hint: "小步改动,每步保持可编译"
  - name: 自测
    expected_tools: [run_command, git_diff]
    hint: "跑 required_checks,确认无回归"
  - name: 提交
    expected_tools: [git_commit, finish]
    hint: "commit message 遵循 Conventional Commits"
```

### 4.6 配置加载流程

```rust
// config/loader.rs
pub fn load() -> Result<Config> {
    let env = EnvConfig::from_env()?;                    // 1. 环境变量
    let project = if let Some(md) = read_devnpc_md()? {  // 2. .devnpc.md
        parse_devnpc_md(&md)?
    } else {
        ProjectConfig::default()
    };
    let roles = load_roles("npc-config/roles/")?;        // 3. 内置角色
    let sops = load_sops("npc-config/sops/")?;
    Ok(Config::merge(env, project, roles, sops))         // 优先级合并
}
```

### 4.7 Config 数据结构

```rust
pub struct Config {
    pub llm: LlmConfig,           // api_key, base_url, model
    pub gitlab: GitlabConfig,     // url, token, project_id
    pub limits: Limits,           // max_iterations, max_ci_retries
    pub project: ProjectConfig,   // .devnpc.md 解析结果
    pub roles: HashMap<String, Role>,
    pub sops: HashMap<String, Sop>,
    pub report: ReportConfig,     // target: pages/artifact/none
}

pub struct ProjectConfig {
    pub sop_mode: SopMode,            // Soft | Strict
    pub forbidden_paths: Vec<String>,
    pub required_checks: Vec<String>,
    pub branch_prefix: String,
    pub guidelines_markdown: String,  // 正文,注入 prompt
}
```

---

## 5. 研发记忆与上下文聚合

### 5.1 记忆来源 (AI Native Git)

决策 Q5 已定: 仅 Git 作为记忆。每次任务从 GitLab 重建上下文,进程无状态。

| 来源 | 获取方式 | 用途 |
|---|---|---|
| 仓库结构 | git ls-tree + 关键文件读取 | Agent 理解项目布局 |
| 目标 Issue | GitLab API: GET /issues/:iid | 任务需求与验收标准 |
| 相关 PR | GitLab API: GET /issues/:iid/related_merge_requests | 历史尝试与讨论 |
| Issue 评论 | GitLab API: GET /issues/:iid/notes | 澄清与补充需求 |
| CI 历史 | GitLab API: GET /projects/:id/pipelines | 已知失败模式 |
| 项目指令 | git show HEAD:.devnpc.md | 规范约束 |
| 最近提交 | git log --oneline -20 | 项目演进脉络 |

### 5.2 上下文聚合器 (memory/context.rs)

```rust
pub struct Context {
    pub repo_tree: RepoTree,
    pub key_files: Vec<KeyFile>,
    pub issue: Issue,
    pub related_prs: Vec<MergeRequest>,
    pub issue_notes: Vec<Note>,
    pub recent_commits: Vec<Commit>,
    pub ci_failures: Vec<CiFailure>,
    pub project_config: ProjectConfig,
}

impl Context {
    pub async fn build(gitlab: &GitlabClient, git: &GitOps, issue_iid: u64) -> Result<Self> {
        let (repo_tree, issue, related_prs, notes, recent_commits, pipelines, project_config) = tokio::try_join!(
            git.build_repo_tree(),
            gitlab.get_issue(issue_iid),
            gitlab.get_related_mrs(issue_iid),
            gitlab.get_issue_notes(issue_iid),
            git.recent_commits(20),
            gitlab.recent_pipelines(5),
            git.read_devnpc_md(),
        )?;
        let key_files = select_key_files(&repo_tree);
        let ci_failures = extract_failures(&pipelines);
        Ok(Self { /* ... */ })
    }
}
```

### 5.3 仓库结构索引 (降 Token 核心)

策略: 只索引结构 + 关键文件摘要,不读全量代码。

```rust
pub struct RepoTree {
    pub entries: Vec<TreeEntry>,
}
pub struct TreeEntry {
    pub path: String,
    pub kind: Kind,         // File | Dir
    pub size: Option<u64>,
}
pub struct KeyFile {
    pub path: String,
    pub summary: String,    // 摘要,非全文
}
```

关键文件选择规则 (6 类):
1. 包管理文件: `Cargo.toml` / `package.json` / `go.mod` / `pyproject.toml`
2. 文档: `README.md` / `.devnpc.md`
3. CI 配置: `.gitlab-ci.yml`
4. 入口文件: `src/main.rs` / `src/lib.rs` (只取前 50 行 + 模块声明)
5. 目录结构: 所有一级目录 + 二级目录名
6. 构建脚本: `Makefile` / `justfile`

摘要生成 (降 Token 关键):
- `Cargo.toml` → 只保留 `[dependencies]` 段
- `README.md` → 前 30 行
- `src/main.rs` → 前 50 行 + `mod` 声明
- 其他源文件 → 不读,留给 Agent 用 AFT `outline` 工具按需查看

### 5.4 CI 失败模式提取

```rust
pub struct CiFailure {
    pub pipeline_id: u64,
    pub job_name: String,
    pub failure_type: FailureType,  // Compile | Test | Lint | Other
    pub root_cause: String,
    pub failed_at: DateTime,
}

fn extract_failures(pipelines: &[Pipeline]) -> Vec<CiFailure> {
    pipelines.iter()
        .filter(|p| p.status == "failed")
        .flat_map(|p| extract_job_failures(p))
        .take(5)  // 最近 5 次失败,防 Token 爆炸
        .collect()
}
```

日志解析 (ci/log_parser.rs 复用):
- 编译错误: 抓 `error[E####]:` 行
- 测试失败: 抓 `test ... FAILED` + `panicked at` 行
- Lint: 抓 `warning:` 行
- 超时: 抓 `timed out` / `killed` 关键字

### 5.5 上下文预算控制

| 部分 | 预算 (token) |
|---|---|
| 仓库结构 | ≤ 500 |
| 关键文件摘要 | ≤ 800 |
| 目标 Issue | ≤ 500 |
| 相关 PR + 评论 | ≤ 500 |
| CI 失败 | ≤ 300 |
| 项目指令 | ≤ 500 |
| **初始上下文总计** | **≤ 3100** |

对标 CodeBuddy 首轮 ~2000 token 目标,预留 buffer。超预算时按优先级截断 (CI 失败 > Issue > 仓库结构 > PR 历史)。

### 5.6 增量上下文 (Agent 循环中)

初始上下文刻意精简,Agent 在循环中用工具按需扩展:

```
初始: 目录树 + 摘要 (3100 token)
  ↓ Agent 调 view_symbol("login.rs", "handle_login")
增量: + handle_login 函数体 (~200 token)
  ↓ Agent 调 search_symbols("password")
增量: + 所有含 password 的符号位置 (~100 token)
```

对比全量塞入: 5000 行 login.rs 全量 ~8000 token,按需 view_symbol 只取目标函数 ~200 token,省 97%。

---

## 6. CI 闭环控制器

### 6.1 闭环流程

```
Agent 完成代码改动 + git push
  ↓
1. 创建 MR (draft) → 2. 等待 Pipeline 触发 (轮询 10s,超时 5min)
  ↓
Pipeline 状态?
  ├─ success → 7. MR ready + 评论验收 + 推送报告
  ├─ running → 继续轮询
  └─ failed → 3. 取失败 Job 日志 → 4. log_parser 提取根因
              ↓
              retry < 3? ─ yes → 5. 喂回 Agent 修复 → push → 回到 2
                       └─ no  → 6. 标记 MR 失败 + 通知人工
```

### 6.2 控制器结构 (ci/controller.rs)

```rust
pub struct CiController {
    gitlab: GitlabClient,
    agent: AgentHandle,
    config: CiConfig,
}

pub struct CiConfig {
    pub poll_interval: Duration,   // 默认 10s
    pub poll_timeout: Duration,    // 默认 5min (等 Pipeline 启动)
    pub pipeline_timeout: Duration,// 默认 30min (等 Pipeline 完成)
    pub max_retries: u8,           // 默认 3 (决策 D5)
}

pub enum CiOutcome {
    Passed { mr_iid: u64, pipeline_id: u64, attempts: u8 },
    Failed { mr_iid: u64, last_error: String, attempts: u8 },
    Timeout { mr_iid: u64, stage: TimeoutStage },
}
```

### 6.3 日志解析器 (ci/log_parser.rs)

```rust
pub enum FailureType {
    Compile, Test, Lint, Build, Timeout, Other,
}

pub struct ParsedFailure {
    pub failure_type: FailureType,
    pub job_name: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub error_message: String,
    pub context_lines: Vec<String>,
}

pub fn parse_log(job_name: &str, log: &str) -> Vec<ParsedFailure> {
    // 去重 + 按严重性排序 + 限 10 条防 Token 爆炸
}
```

多语言支持 (按项目类型识别): Rust (MVP) / Go / Python / Node (P9 扩展)。

### 6.4 修复循环的 Agent 交互

复用原 Agent 上下文 (保留对话历史与已读符号,避免重复读码省 Token)。

```rust
async fn request_fix(&self, failures: &[ParsedFailure], ctx: &AgentContext) -> Result<()> {
    let fix_task = Task::CiFix {
        failures: failures.to_vec(),
        original_task: ctx.original_task.clone(),
    };
    let result = self.agent.run(&fix_task, ctx).await?;
    // Agent 修复后通过 git_commit + push 工具自动推送
    Ok(())
}
```

提示词补充 (CI 修复任务): 注入失败类型、文件、行号、错误信息、上下文行、原始任务。

### 6.5 超时与兜底

| 阶段 | 超时 | 兜底动作 |
|---|---|---|
| 等 Pipeline 启动 | 5 min | 标记异常,通知人工 |
| 等 Pipeline 完成 | 30 min | 标记超时,通知人工 |
| 修复重试 | 3 次 (D5) | 标记 MR 失败,评论原因 + @maintainer |
| 单次 Agent 修复循环 | 20 次迭代 | 视为修复失败,消耗一次重试 |

### 6.6 MR 状态管理

```
Draft (创建,WIP) → CI 修复中 → Ready (成功) / Failed (放弃)
```

- 创建时: MR draft=true,title 加 [WIP]
- CI 修复中: 评论 "CI 修复中 (attempt 1/3)..."
- 成功: MR draft=false,评论 "CI 通过,请 review"
- 失败: 评论 "CI 修复失败 (3次),最后错误: ...,请人工介入"

---

## 7. 多 NPC 协同与 Team 编排

P7 实施,MVP (P0-P5) 不涉及。

### 7.1 Team 模型

```yaml
# npc-config/teams/feature-team.yml
name: feature-team
description: 功能开发团队,PM 拆需求 + 开发 + 测试
npcs:
  - role: pm
    sop: requirement-decompose
  - role: developer
    sop: feature
  - role: tester
    sop: test-gen
handoff:
  - from: pm
    to: [developer, tester]
    trigger: pm 发出 "decomposed" 信号
  - from: developer
    to: tester
    trigger: developer 发出 "implemented" 信号
merge:
  strategy: single-mr  # 所有 NPC 产出汇入同一 MR
```

### 7.2 通信总线 (决策 Q7: GitLab 评论)

NPC 间通过 GitLab Issue/MR 评论传递结构化消息,每次评论带协议头。

```
[devnpc:handoff]
from: pm
to: [developer, tester]
signal: decomposed
payload:
  subtasks:
    - id: 1
      assignee: developer
      desc: "实现 /api/login 密码转义"
[/devnpc:handoff]
```

优势: 可审计、持久化、人工可介入。代价: 延迟高 (轮询 10s),但研发流程对秒级延迟不敏感。

### 7.3 编排器 (team/orchestrator.rs)

```rust
pub struct Orchestrator {
    gitlab: GitlabClient,
    npcs: HashMap<String, NpcRunner>,
    team_def: TeamDef,
    issue_iid: u64,
}

impl Orchestrator {
    pub async fn run(&self, goal: &str) -> Result<TeamOutcome> {
        // 1. PM 拆解需求
        let handoff = self.run_npc("pm", goal).await?;
        self.post_handoff(&handoff).await?;
        // 2. 并行执行开发 + 测试
        let (dev_result, test_result) = tokio::join!(
            self.run_npc_with_handoff("developer", &handoff),
            self.run_npc_with_handoff("tester", &handoff),
        );
        // 3. 联调 + 汇入同一 MR + CI 闭环
        let mr_iid = self.create_team_mr(&dev_result?, &test_result?).await?;
        CiController::new(/*...*/).run(mr_iid, /*...*/).await
    }
}
```

### 7.4 冲突处理

| 场景 | 处理 |
|---|---|
| 改不同文件 | 各自在子分支工作,合并时无冲突 |
| 改同文件不同函数 | AFT edit_symbol 按符号定位,git 自动合并 |
| 改同函数 | 后提交者 git push 失败 → git_pull --rebase → 冲突则报告 PM 重新分配 |

分支策略:
```
main
 └─ npc/team-<issue-id>/        # Team 集成分支
     ├─ npc/pm-<issue-id>
     ├─ npc/dev-<issue-id>
     └─ npc/test-<issue-id>
```

---

## 8. 事件触发与 GitLab CI 集成

### 8.1 触发场景

| 触发场景 | CI 变量信号 | MVP |
|---|---|---|
| MR 评论 @devnpc | `CI_MERGE_REQUEST_IID` + MR note | ✅ |
| 手动触发 | `CI_PIPELINE_SOURCE=web` + `DEVNPC_TASK` | ✅ |
| Issue 评论 @devnpc | Issue note | P5 |
| Issue 创建 @devnpc | Issue 描述含 @devnpc | P5 |

### 8.2 触发解析器 (trigger/parser.rs)

```rust
pub enum Trigger {
    IssueTask { issue_iid: u64, task: TaskSpec },
    MrTask { mr_iid: u64, task: TaskSpec },
    Manual { task: TaskSpec },
    None,
}

pub struct TaskSpec {
    pub kind: TaskKind,  // Implement | Fix | Test | Refactor | Review
    pub description: String,
    pub target_issue: Option<u64>,
    pub acceptance_criteria: Vec<String>,
}
```

### 8.3 @devnpc 指令语法

自然语言 + 关键字识别,无需严格命令式:

```
@devnpc 修复 #42 的登录 bug              → Fix, target=42
@devnpc 为 login 接口增加单元测试         → Test
@devnpc 把 password 改用 bcrypt 加密     → Refactor
@devnpc 实现 #58 描述的导出功能           → Implement, target=58
@devnpc review !37                       → Review, target_mr=37
@devnpc stop                             → 控制指令 (终止当前任务)
```

关键字: 修复/fix/bug → Fix; 测试/test → Test; 重构/refactor → Refactor; 实现/implement → Implement; review → Review; stop → 控制指令。

### 8.4 .gitlab-ci.yml 集成模板

```yaml
stages:
  - test
  - npc

test:
  stage: test
  script:
    - cargo test

devnpc:
  stage: npc
  image: registry.yourcompany.com/devnpc:0.1  # 预构建镜像
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
      when: on_stop
    - if: $CI_PIPELINE_SOURCE == "web"
      when: manual
  variables:
    DEVNPC_API_KEY: $DEVNPC_API_KEY
    DEVNPC_BASE_URL: "https://api.deepseek.com/v1"
    DEVNPC_MODEL: "deepseek-chat"
    GITLAB_TOKEN: $DEVNPC_GITLAB_TOKEN
  before_script:
    - git config --global user.email "devnpc-bot@example.com"
    - git config --global user.name "devnpc"
    - git config --global safe.directory "*"
  script:
    - devnpc run
  artifacts:
    when: always
    paths:
      - .devnpc-report/
    expire_in: 7 days
```

### 8.5 镜像化部署 (Dockerfile)

```dockerfile
FROM rust:1.97-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y git ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/devnpc /usr/local/bin/
ENTRYPOINT ["devnpc"]
```

### 8.6 沙箱安全约束

| 约束 | 实现 |
|---|---|
| 文件系统 | Agent 工具限制在 `$CI_PROJECT_DIR` 内,path traversal 检查 |
| Git 写权限 | 只能 push 到 `npc/*` 分支,配置 GitLab 分支保护 `main` 只允许人工 merge |
| 网络出站 | 默认允许 GitLab + LLM API;CI Runner 可配网络白名单 |
| 密钥隔离 | `GITLAB_TOKEN` 仅在 devnpc 作业可见 (protected env) |
| 命令黑名单 | `run_command` 工具拦截 `rm -rf /`、`dd`、`mkfs` 等 |

### 8.7 评论触发机制

- **方案 A (MVP,零组件)**: 评论后手动重跑 devnpc 作业,parser 扫描 MR notes 找最新 @devnpc
- **方案 B (P5,全自动)**: GitLab webhook → Pipeline trigger API 自动触发 devnpc 作业

---

## 9. 运维报告与可观测性

### 9.1 报告内容

每次 NPC 任务完成生成自包含 HTML 报告:
- 📊 概览: 状态/耗时/Token/LLM调用/工具调用/CI重试
- 📝 任务: 原始指令 + 解析结果
- 🔄 执行轨迹: 时间线 (LLM思考/工具调用/CI状态)
- 💰 成本: 模型/token 单价/合计
- 📦 产出: MR/改动/CI/报告链接

### 9.2 轨迹采集 (report/collector.rs)

通过 `tracing` 事件订阅,不侵入业务逻辑。

```rust
// 各模块埋点
tracing::info!(target: "devnpc.agent", iter=3, "llm_called");
tracing::info!(target: "devnpc.tool", name="edit_symbol", "tool_called");
tracing::info!(target: "devnpc.ci", pipeline=201, status="failed", "pipeline_update");

// report 模块订阅
pub struct TrajectoryCollector {
    events: Arc<Mutex<Vec<TrajectoryEvent>>>,
}
```

### 9.3 报告生成 (report/html.rs)

用 `askama` 编译期模板生成单文件 HTML (CSS/JS 内联,无外部依赖)。

### 9.4 报告推送 (report/publisher.rs)

```rust
pub enum ReportTarget {
    GitlabPages,
    Artifact,   // MVP 默认,零配置
    MrAttachment,
    None,
}
```

MVP 选 Artifact (CI 自动收集)。Pages 需 GitLab Pages 启用,按需切。

### 9.5 报告链接反馈

任务完成后在 MR 评论贴报告链接 + 验收摘要。

### 9.6 可观测性补强

运行时日志走 `tracing`,支持 JSON (ELK 采集) 或 pretty (人读) 格式。CI 日志在 GitLab 作业页面直接可查,HTML 报告做事后总结。

日志层级: ERROR (任务失败) / WARN (SOP偏离) / INFO (LLM/工具/Pipeline) / DEBUG (提示词/参数) / TRACE (HTTP原文)。

---

## 10. 错误处理、测试策略与实施阶段

### 10.1 错误处理

统一错误类型 (error.rs),库层用 `DevnpcError`,CLI 层用 `anyhow`。

```rust
#[derive(thiserror::Error, Debug)]
pub enum DevnpcError {
    #[error("配置错误: {0}")]
    Config(String),
    #[error("环境变量缺失: {var}")]
    MissingEnv { var: String },
    #[error("GitLab API 错误: {status} {body}")]
    GitlabApi { status: u16, body: String },
    #[error("GitLab 资源不存在: {resource}")]
    GitlabNotFound { resource: String },
    #[error("Git 命令失败: {cmd} (exit {code})")]
    GitCommand { cmd: String, code: i32 },
    #[error("分支保护: 不允许操作 {branch}")]
    BranchProtected { branch: String },
    #[error("LLM 调用失败: {0}")]
    Llm(String),
    #[error("Agent 达到迭代上限 ({max})")]
    MaxIterations { max: u32 },
    #[error("任务被取消")]
    Cancelled,
    #[error("CI 修复失败,重试 {attempts} 次未通过")]
    CiFixExhausted { attempts: u8 },
    #[error("Pipeline 超时 ({stage})")]
    PipelineTimeout { stage: String },
    #[error("工具调用错误: {tool}: {msg}")]
    Tool { tool: String, msg: String },
    #[error("路径越界: {path} 不在 workspace 内")]
    PathTraversal { path: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, DevnpcError>;
```

错误策略:
- 可重试错误 (LLM 网络抖动、GitLab 5xx) → 指数退避重试 3 次
- 不可恢复错误 (配置缺失、权限不足) → 立即退出 + 清晰报错

### 10.2 测试策略

| 层级 | 范围 | 工具 | 覆盖目标 |
|---|---|---|---|
| 单元测试 | 纯函数 (日志解析、路径检查、SOP 偏离检测) | `#[test]` | 核心逻辑 80%+ |
| 集成测试 | 模块间协作 (Agent+Tools、CiController) | `#[tokio::test]` + mockall | 关键路径 |
| 端到端 | 完整闭环 (真实 GitLab + LLM) | 手动 + CI 冒烟 | MVP 验收 |

Mock 策略: GitLab 客户端与 LLM 客户端均用 trait 抽象,测试时返回固定响应。

关键测试用例:
- `parse_compile_error_extracts_file_line`
- `path_traversal_blocked`
- `ci_controller_retries_on_failure`
- `agent_loop_finishes_on_no_tool_call`
- `sop_strict_mode_blocks_deviation`

端到端冒烟: `devnpc run --task "echo hello" --dry-run` (不真正改码,验证启动链路)。

### 10.3 实施阶段总览 (Checkpoint 制)

| 阶段 | 内容 | 验收标准 | 优先级 |
|---|---|---|---|
| P0 | 骨架: Cargo.toml + 模块文件 + 依赖编译 | `cargo build` 成功 | 高 |
| P1 | 配置系统 + GitLab API 客户端 | 能读取 Issue/MR/Pipeline | 高 |
| P2 | Git 操作 + 研发记忆聚合 | 能 clone 仓库并输出上下文摘要 | 高 |
| P3 | Agent 核心 + AFT 工具集 + ReAct 循环 | 本地能跑通"读文件→改文件→commit" | 高 |
| P4 | CI 闭环 (MR→Pipeline→修复) | 端到端跑通一次 Issue→MR→CI 通过 | 高 |
| P5 | 事件触发 + .gitlab-ci.yml + 镜像 | @devnpc 触发完整闭环 | 高 |
| P6 | NPC 角色系统 (Role/SOP) + Skill 评估 | 可加载自定义角色 | 中 |
| P7 | Team 多 NPC 协同 | PM/DEV/TEST 分工跑通 | 中 |
| P8 | 模型路由 + Token 优化 | 简单/复杂任务分流 | 中 |
| P9 | 多语言日志解析 + 文档 + 示例 | 可上手使用 | 中 |

MVP = P0–P5 (单 NPC 完整闭环 + 报告),P6–P9 增强。

### 10.4 依赖清单定稿

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
futures = "0.3"
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "stream"] }
rig-core = "0.10"
agent-file-tools = "0.49"
tree-sitter = "0.25"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
clap = { version = "4", features = ["derive", "env"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
thiserror = "2"
anyhow = "1"
askama = "0.12"
url = "2"
chrono = { version = "0.4", features = ["serde"] }
regex = "1"
dotenvy = "0.15"
async-trait = "0.1"

[dev-dependencies]
tokio-test = "0.4"
mockall = "0.13"
tempfile = "3"
```

注: 移除 `gitlab` crate 和 `async-openai` —— rig 内部已封装 LLM 客户端,GitLab API 自建 reqwest 客户端即可。

### 10.5 目录结构终稿

```
devnpc/
├── Cargo.toml
├── Dockerfile
├── .gitlab-ci.yml.example
├── npc-config/
│   ├── roles/{developer,tester,pm}.yml
│   ├── sops/{bugfix,feature,test-gen}.yml
│   └── teams/feature-team.yml
├── templates/
│   └── report.html.askama
└── src/
    ├── main.rs
    ├── lib.rs
    ├── error.rs
    ├── config/{mod,env,loader}.rs
    ├── gitlab_api/{client,issues,mrs,pipelines,notes,repo}.rs
    ├── git/ops.rs
    ├── memory/{context,repo_index}.rs
    ├── agent/{loop,prompt,llm_client,sop}.rs
    ├── tools/{mod,file_io,shell,git_tool,gitlab_tool}.rs
    ├── ci/{controller,log_parser}.rs
    ├── npc/{role,sop,runner}.rs
    ├── team/{orchestrator,comm}.rs
    ├── trigger/parser.rs
    └── report/{collector,html,publisher}.rs
```

---

## 风险与应对

| 风险 | 应对 |
|---|---|
| rig-core 版本/API 变动 | 锁定版本,封装 llm_client.rs 隔离 |
| AFT 与 tree-sitter 版本兼容 | 锁定版本,封装 file_io.rs 隔离 |
| LLM 工具调用协议各家差异 | rig 抽象层屏蔽,测试 DeepSeek/GLM/OpenAI 兼容性 |
| CI 修复死循环烧 Token | 硬重试上限 3 + 迭代上限 20 + Token 预算守卫 + 失败兜底人工通知 |
| 大仓库上下文爆炸 | 按需读取 + 目录树优先 + 3100 token 初始预算 + AFT 符号级读取 |
| Windows 开发/CI 差异 | 用系统 git 命令而非 libgit2,避免平台问题 |
| 评论触发需手动重跑 | MVP 接受,P5 加 webhook 全自动 |

---

## 附录: 设计过程记录

本设计通过 brainstorming 技能,经 10 节分节评审 + 逐节用户确认形成。关键决策点均经用户确认:
- 8 个澄清问题 (范围/GitLab/LLM/认证/记忆/指令/通信/路由)
- 2 个小决策 (重试上限/分支命名)
- 6 个架构选型 (Agent架构/LLM抽象/code-agent框架/运维页面/HTML生成/Skill抽象)
