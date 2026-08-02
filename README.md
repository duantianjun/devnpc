# devnpc

基于 GitLab 的企业级研发流程 AI 智能体。

devnpc 监听 GitLab Issue/MR 中的 `@devnpc` 提及,在 CI 内自主完成"读上下文 → 改代码 → 提交 → 跟踪 CI → 修复失败"的闭环,并把执行轨迹沉淀为运维报告。

## 技术栈

| 层 | 选型 | 说明 |
|----|------|------|
| 语言 | Rust 2024 | 静态分发、零成本抽象、编译期错误检查 |
| 异步运行时 | tokio (full) | async/await + 超时 + 进程 |
| Agent 框架 | adk-rust (zavora-ai) | LlmAgent + FunctionTool + Runner + 工作流编排 |
| 模型提供商 | deepseek / openai / anthropic / gemini | 多模型路由,按任务类型选择 |
| GitLab API | reqwest + 自建 client | REST API v4 |
| 代码工具 | agent-file-tools + tree-sitter | AST 级代码感知,省 token |
| CLI | clap (derive) | `devnpc run / config / info` |
| 序列化 | serde + serde_json + serde_yaml | 配置 + 消息 + 工具参数 |
| 报告 | 自建 HTML 模板引擎 | 运行轨迹 + 成本估算 |
| 日志 | tracing + tracing-subscriber | 结构化日志 |
| 错误 | thiserror + anyhow | 库层枚举 + CLI 层透明 |

## 架构

```
┌─────────────────────────────────────────────────────────────┐
│  CLI (main.rs)  devnpc run / config / info                  │
├─────────────────────────────────────────────────────────────┤
│  trigger  ← @devnpc 提及解析                                 │
│  memory   ← Git 仓库树 + Issue/PR/CI 记忆聚合                │
│  adapter  ← adk-rust 框架适配层                              │
│  │  tools.rs     ← 业务工具 → FunctionTool 包装              │
│  │  callbacks.rs ← SOP 检测 + 轨迹记录                       │
│  │  context.rs   ← 业务上下文 → Session 注入                 │
│  │  provider.rs  ← 多模型提供商配置                          │
│  │  file_io.rs   ← 带路径安全检查的文件 I/O                   │
│  ci       ← Pipeline 失败 → 日志解析 → 修复闭环              │
│  report   ← 轨迹采集 + HTML 生成 + 发布                      │
├─────────────────────────────────────────────────────────────┤
│  config   ← env > .devnpc.md > 默认值  (三层合并)            │
│  gitlab_api ← REST v4 客户端                                 │
│  git      ← GitOps (系统 git 命令封装)                       │
└─────────────────────────────────────────────────────────────┘
```

**核心设计决策**:
- **adk-rust 框架**:基于 zavora-ai/adk-rust 的 LlmAgent + FunctionTool + Runner,替代自研 ReAct 循环
- **SOP 双层约束**:通过 `before_tool_callback` 实现 soft 模式(偏离只警告) + strict 模式(偏离即阻断)
- **唯一副作用出口**:所有写操作走 FunctionTool 包装层,便于审计与沙箱化
- **研发记忆**:任务执行前聚合 Git 仓库树 + 关键文件摘要 + Issue/PR/CI 历史,降低 LLM 上下文成本
- **集中配置**:三层来源 (env > .devnpc.md > 内置默认值),统一管理

## 模块说明

| 模块 | 职责 |
|------|------|
| [src/config/](src/config/) | 三层配置合并(env > .devnpc.md > 默认) |
| [src/gitlab_api/](src/gitlab_api/) | GitLab REST v4 客户端(9 个 API 方法) |
| [src/git/](src/git/) | GitOps 同步封装(`std::process::Command`) |
| [src/memory/](src/memory/) | 研发记忆聚合(Context + repo_index) |
| [src/adapter/](src/adapter/) | adk-rust 框架适配层(工具、回调、上下文、提供商) |
| [src/ci/](src/ci/) | CI 闭环控制器 + 日志解析 |
| [src/trigger/](src/trigger/) | `@devnpc` 提及解析 |
| [src/report/](src/report/) | 轨迹采集 + HTML 报告 + 发布 |

## 功能一览

### 核心能力

| 能力 | 说明 |
|------|------|
| **AI 自动编码** | 理解项目上下文,自动修改代码、修复 bug、实现功能 |
| **CI 闭环** | 自动创建 Draft MR → 轮询 Pipeline → 失败时解析日志 → 修复重试 |
| **多触发方式** | MR/Issue 评论 `@devnpc` 触发,或命令行手动触发 |
| **多模型路由** | 支持 DeepSeek/OpenAI/Anthropic/Gemini,简单/复杂任务可用不同模型 |
| **9 种语言支持** | Rust、Java、Python、JavaScript、TypeScript、Go、C、C++ 的 AST 级代码感知 |

### 工具列表

| 工具 | 功能 |
|------|------|
| `read_file` | 读取文件内容(限前 N 行防 token 溢出) |
| `write_file` | 写入文件(全量覆盖,自动创建目录) |
| `list_files` | 列出目录内容 |
| `run_command` | 执行白名单命令(`cargo/rustc/make/just/fmt/clippy/echo`),黑名单拦截(`rm/mv/cp/curl/wget/ssh/scp`),可配置超时 |
| `git_diff` | 查看工作区相对 HEAD 的未提交改动 |
| `git_commit` | 提交所有改动 |
| `create_mr_note` | 在 MR 上评论执行结果 |
| `aft_outline` | 列出文件所有顶层符号(函数、结构体、类等) |
| `aft_view_symbol` | 查看指定符号的完整定义源码 |
| `aft_edit_symbol` | 替换指定符号的完整定义(替换后语法验证) |
| `aft_search_symbols` | 在工作区按正则搜索符号名 |
| `aft_ast_replace` | 在文件中按正则查找替换,替换后验证语法 |

### GitLab API 能力

| API | 功能 |
|-----|------|
| 获取 Issue/MR 详情 | 读取 Issue 和 MR 的标题、描述、状态 |
| 创建/更新 MR | 创建 Draft MR,移除 Draft 状态 |
| 获取 Pipeline 与 Job | 查询 Pipeline 状态、Job 列表、Job 原始日志 |
| 评论管理 | 读取 Issue/MR 评论,发表评论 |
| 关联查询 | 获取 Issue 关联的 MR、最近 Pipeline |

### 智能上下文(研发记忆)

Agent 执行前自动聚合以下信息,降低 LLM 上下文成本:

| 信息 | 来源 |
|------|------|
| 仓库目录树 | `git ls-tree` |
| 关键文件摘要 | Cargo.toml、README.md、main.rs 等 |
| Issue 详情 | GitLab API |
| 关联 MR | GitLab API |
| 评论历史 | GitLab API |
| 最近提交记录 | `git log` |
| CI 失败历史 | 最近 Pipeline 失败记录 |
| 项目规范 | `.devnpc.md` 配置 |

### 配置系统

三层来源合并(优先级: 环境变量 > `.devnpc.md` > 内置默认值):

- **LLM 提供商**: provider/model/api_key/base_url
- **GitLab 连接**: url/token/project_id
- **命令安全**: 白名单/黑名单/超时
- **CI 闭环**: 轮询间隔/超时/最大重试
- **SOP 约束**: soft(偏离警告) / strict(偏离即阻断)
- **模型路由**: 简单/复杂任务使用不同模型
- **报告发布**: Artifact / Pages / 不发布

### 报告系统

执行完成后自动生成 HTML 报告,包含:

- 执行状态与耗时
- Token 消耗与成本估算
- LLM 调用次数与工具调用轨迹
- CI 重试次数
- 完整的执行事件流

## GitLab 集成

### 1. 部署

构建 Docker 镜像并推送到 GitLab Registry:

```bash
docker build -t registry.example.com/devnpc:latest .
docker push registry.example.com/devnpc:latest
```

### 2. GitLab CI 配置

复制 [.gitlab-ci.yml.example](.gitlab-ci.yml.example) 到项目根目录的 `.gitlab-ci.yml`:

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
  image: registry.example.com/devnpc:latest
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
      when: on_stop
    - if: $CI_PIPELINE_SOURCE == "web"
      when: manual
  variables:
    DEVNPC_BASE_URL: "https://api.deepseek.com/v1"
    DEVNPC_MODEL: "deepseek-chat"
  script:
    - devnpc run
  artifacts:
    paths:
      - .devnpc-report/
```

### 3. CI 变量配置

在 GitLab 项目 **Settings → CI/CD → Variables** 中添加:

| 变量 | 说明 | 必填 |
|------|------|------|
| `DEVNPC_API_KEY` | LLM API Key | 是 |
| `DEVNPC_GITLAB_TOKEN` | GitLab Personal Access Token (需 `api` 权限) | 是 |

### 4. 触发方式

#### 方式 A: 在 MR 评论中 @devnpc (推荐)

在 MR 评论中写 `@devnpc` 后跟任务描述:

```
@devnpc 修复用户登录模块的 token 刷新 bug
```

devnpc 自动检测 `CI_MERGE_REQUEST_IID` 环境变量,解析最新评论中的提及。

#### 方式 B: 在 Issue 评论中 @devnpc

在 Issue 评论中写 `@devnpc` 后跟任务描述。需要确保 CI 环境变量 `CI_ISSUE_IID` 已设置。

#### 方式 C: 手动触发 (调试)

在 GitLab CI 中手动运行 pipeline,或本地命令行:

```bash
devnpc run --task "添加用户登录接口的单元测试"
```

### 5. 执行流程

```
@devnpc 提及
    ↓
trigger 解析 → 读取 MR/Issue 上下文
    ↓
Context 构建 → Git 仓库树 + 关键文件摘要 + CI 历史
    ↓
LlmAgent 执行 → 读上下文 → 改代码 → 验证编译
    ↓
Git 提交 → 创建 Draft MR
    ↓
CI 控制器 → 轮询 pipeline → 日志解析 → 修复失败 → 重试
    ↓
报告生成 → 发布 HTML 报告 → 评论 MR
```

1. devnpc 读取 MR/Issue 的评论,解析任务描述
2. 构建研发记忆(仓库树结构、关键文件摘要、CI 失败历史)
3. LlmAgent 开始执行: 读取上下文 → 修改代码 → 编译验证
4. 修改完成后 `git commit` + 创建 Draft MR
5. CI 控制器轮询 pipeline 状态,失败时自动解析日志、修复、重试
6. 生成 HTML 执行报告,发布到 Pages 或 artifact,并在 MR 中评论结果

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

# 本地执行任务
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

- **实现流程**:先写 plan 到 `docs/superpowers/plans/` → 确认设计偏离 → 执行 → 验证
- **验收门槛**:`cargo test --all` 全通过 + `cargo clippy --all-targets -- -D warnings` 零警告 + `cargo build --release` 成功
- **commit 规范**:`feat: <描述>` / `fix: <描述>` / `chore: <描述>` / `docs: <描述>`

## 仓库结构

```
devnpc/
├── src/
│   ├── adapter/        # adk-rust 框架适配层
│   ├── ci/             # CI 闭环控制器
│   ├── config/         # 配置系统 (三层合并)
│   ├── git/            # GitOps 封装
│   ├── gitlab_api/     # GitLab REST v4 客户端
│   ├── memory/         # 研发记忆聚合
│   ├── report/         # 运维报告
│   ├── trigger/        # 事件触发
│   ├── error.rs        # 统一错误类型
│   ├── lib.rs          # 模块树
│   └── main.rs         # CLI 入口 (LlmAgent + Runner)
├── npc-config/
│   ├── roles/          # NPC 角色定义 (developer/tester/pm)
│   ├── sops/           # 标准操作流程 (feature/bugfix/test-gen)
│   └── teams/          # 团队编排配置
├── docs/superpowers/   # 设计文档与计划
├── Dockerfile
├── .gitlab-ci.yml.example
├── Cargo.toml
└── README.md
```

## License

MIT