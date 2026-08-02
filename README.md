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