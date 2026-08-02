# devnpc

基于 GitLab 的企业级研发流程 AI 智能体(对标 CodeBuddy NPC)。

devnpc 监听 GitLab Issue/MR 中的 `@devnpc` 提及,在 CI 内自主完成"读上下文 → 改代码 → 提交 → 跟踪 CI → 修复失败"的闭环,并把执行轨迹沉淀为运维报告。

## 技术栈

| 层 | 选型 | 说明 |
|----|------|------|
| 语言 | Rust 2021 | 静态分发、零成本抽象、编译期错误检查 |
| 异步运行时 | tokio (full) | async/await + 超时 + 进程 |
| HTTP | reqwest (rustls) | 直连 OpenAI 兼容 Chat Completions API |
| LLM 抽象 | 自建客户端 + ModelRouter | 单 provider 直连,支持按任务类型路由 |
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
│  agent    ← ReAct 循环 (LLM ↔ Tool) + SOP 软约束            │
│  tools    ← 8 自建工具 + 5 AFT 工具                           │
│  ci       ← Pipeline 失败 → 日志解析 → 修复闭环              │
│  npc      ← Role/SOP 加载 + 单 NPC 执行器                    │
│  team     ← 多 NPC 编排 + GitLab 评论总线                    │
│  report   ← 轨迹采集 + HTML 生成 + 发布                      │
├─────────────────────────────────────────────────────────────┤
│  config   ← env > .devnpc.md > 默认值  (三层合并)            │
│  gitlab_api ← REST v4 客户端                                 │
│  git      ← GitOps (系统 git 命令封装)                       │
└─────────────────────────────────────────────────────────────┘
```

**核心设计决策**:
- **混合架构**:自建 ReAct 循环 + AFT(代码工具层) + 可选 ModelRouter 模型路由
- **SOP 双层约束**:soft 模式(偏离只警告) + strict 模式(偏离即阻断)
- **唯一副作用出口**:所有写操作走 `tools` 模块,便于审计与沙箱化
- **研发记忆**:任务执行前聚合 Git 仓库树 + 关键文件摘要 + Issue/PR/CI 历史,降低 LLM 上下文成本
- **集中配置**:三层来源 (env > .devnpc.md > 内置默认值),统一管理

## 模块说明

| 模块 | 职责 |
|------|------|
| [src/config/](src/config/) | 三层配置合并(env > .devnpc.md > 默认) |
| [src/gitlab_api/](src/gitlab_api/) | GitLab REST v4 客户端(9 个 API 方法) |
| [src/git/](src/git/) | GitOps 同步封装(`std::process::Command`) |
| [src/memory/](src/memory/) | 研发记忆聚合(Context + repo_index) |
| [src/agent/](src/agent/) | ReAct 循环 + LLM 客户端 + 消息类型 + prompt + SOP |
| [src/tools/](src/tools/) | 8 个自建工具 + 5 个 AFT 工具 + ToolRegistry |
| [src/ci/](src/ci/) | CI 闭环控制器 + 日志解析 |
| [src/trigger/](src/trigger/) | `@devnpc` 提及解析 |
| [src/npc/](src/npc/) | Role/SOP 加载 + 单 NPC 执行器 |
| [src/team/](src/team/) | 多 NPC 编排 + 评论总线 |
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
│   ├── agent/          # ReAct 循环 + LLM 客户端 + 消息 + prompt + SOP
│   ├── ci/             # CI 闭环控制器
│   ├── config/         # 配置系统 (三层合并)
│   ├── git/            # GitOps 封装
│   ├── gitlab_api/     # GitLab REST v4 客户端
│   ├── memory/         # 研发记忆聚合
│   ├── npc/            # NPC 角色系统
│   ├── report/         # 运维报告
│   ├── team/           # 多 NPC 协同
│   ├── tools/          # 自建工具 + AFT 工具 + ToolRegistry
│   ├── trigger/        # 事件触发
│   ├── error.rs        # 统一错误类型
│   ├── lib.rs          # 模块树
│   └── main.rs         # CLI 入口
├── npc-config/
│   ├── roles/          # NPC 角色定义 (developer/tester/pm)
│   ├── sops/           # 标准操作流程 (feature/bugfix/test-gen)
│   └── teams/          # 团队编排配置
├── docs/superpowers/plans/  # 实现计划文档
├── Dockerfile
├── .gitlab-ci.yml.example
├── Cargo.toml
└── README.md
```

## License

MIT