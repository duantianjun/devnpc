use std::sync::Arc;

use clap::{Parser, Subcommand};

use adk_rust::agent::LlmAgentBuilder;

use devnpc::adapter::agents::{build_code_agent, build_fix_agent, build_pm_agent, build_review_agent};
use devnpc::adapter::callbacks::DevnpcCallbacks;
use devnpc::adapter::context::{build_initial_state, create_session_service};
use devnpc::adapter::provider::{create_complex_model, create_simple_model};
use devnpc::adapter::tools::create_all_tools;
use devnpc::ci::controller::{CiController, CiOutcome, FixHandler};
use devnpc::ci::log_parser::ParsedFailure;
use devnpc::config::npc_config::NpcConfig;
use devnpc::config::Config;
use devnpc::error::Result;
use devnpc::git::ops::GitOps;
use devnpc::gitlab_api::client::GitlabClient;
use devnpc::gitlab_api::GitlabApi;
use devnpc::memory::context::Context;
use devnpc::report::collector::{CostEstimate, ReportData, Trajectory, TrajectoryEvent, TrajectorySummary};
use devnpc::report::publisher;
use devnpc::trigger::parser::{classify_task, parse_mention_with_pattern, Trigger};

/// devnpc - 基于 GitLab 的企业级研发流程 AI 智能体
#[derive(Parser, Debug)]
#[command(name = "devnpc", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// 运行 NPC 任务 (CI 内调用)
    Run {
        /// 手动指定任务描述 (调试用)
        #[arg(long)]
        task: Option<String>,

        /// 干跑模式,不真正改码 (冒烟测试用)
        #[arg(long)]
        dry_run: bool,
    },

    /// 启动 Webhook 服务器,监听 GitLab 事件实时触发任务
    Serve {
        /// 监听端口 (覆盖 DEVNPC_WEBHOOK_PORT)
        #[arg(long)]
        port: Option<u16>,

        /// 监听地址 (覆盖 DEVNPC_WEBHOOK_HOST)
        #[arg(long)]
        host: Option<String>,
    },

    /// 打印当前配置 (调试用)
    Config,

    /// 打印版本与构建信息
    Info,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // 启动时自动加载当前目录 .env (不覆盖已存在的环境变量)
    // 这样本地运行无需手动 Get-Content .env | Set-Item 逐条加载
    devnpc::config::env::load_env_file();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run { task, dry_run }) => run(task.as_deref(), dry_run).await,
        Some(Commands::Serve { port, host }) => serve(port, host).await,
        Some(Commands::Config) => print_config(),
        Some(Commands::Info) | None => {
            print_info();
            Ok(())
        }
    }
}

/// Webhook 服务器模式
///
/// 启动 axum HTTP 服务器监听 GitLab webhook 事件。
/// 收到 Note 事件中的 @devnpc 提及时,自动触发任务执行。
///
/// 与 `run` 命令的区别:
/// - `run`: 单次执行 (CI 内调用或手动 --task)
/// - `serve`: 长驻进程,实时响应 webhook 事件,适合部署在服务器上
async fn serve(port_override: Option<u16>, host_override: Option<String>) -> Result<()> {
    let mut config = Config::load()?;

    // CLI 参数覆盖配置
    if let Some(port) = port_override {
        config.webhook.port = port;
    }
    if let Some(host) = host_override {
        config.webhook.host = host;
    }
    config.webhook.enabled = true;

    if config.webhook.secret.is_empty() {
        tracing::warn!(
            "DEVNPC_WEBHOOK_SECRET 未设置,GitLab webhook 将无 secret 校验 (生产环境建议配置)"
        );
    }

    let (handle, mut receiver) = devnpc::trigger::webhook::start_server(&config.webhook, &config.trigger.mention_regex)
        .await
        .map_err(|e| devnpc::error::DevnpcError::Config(format!("Webhook 服务器启动失败: {e}")))?;

    tracing::info!("Webhook 服务器已启动,等待 GitLab 事件...");

    // 主循环: 接收 webhook 触发事件,为每个任务启动独立的 run 执行
    while let Some(trigger) = receiver.recv().await {
        let task_desc = trigger.task.description.clone();
        let task_kind = format!("{:?}", trigger.task.kind);
        let target_iid = trigger.target_iid;
        let source = trigger.source.clone();

        tracing::info!(
            source = %source,
            target_iid = target_iid,
            kind = %task_kind,
            "收到 webhook 触发,启动任务执行"
        );

        // 在独立 task 中执行,不阻塞 webhook 接收
        tokio::spawn(async move {
            // 构造 --task 参数形式,复用 run 函数
            if let Err(e) = run(Some(&task_desc), false).await {
                tracing::error!(
                    target_iid = target_iid,
                    error = %e,
                    "webhook 触发的任务执行失败"
                );
            }
        });
    }

    // receiver 关闭 (send 端 drop) 时退出
    let _ = handle.await;
    Ok(())
}

/// 主运行流程 (基于 adk-rust):
///
/// 1. 加载配置
/// 2. 创建 GitLab 客户端
/// 3. dry_run 模式: 打印配置后退出
/// 4. 解析触发源 (CI env vars / --task)
/// 5. 构建上下文 (Context::build)
/// 6. 创建 LlmAgent 并执行
/// 7. 若分支已推送: CI 控制器 → 创建 MR → 轮询 pipeline → 修复
/// 8. 生成报告 → 发布 → 评论 MR
async fn run(task: Option<&str>, dry_run: bool) -> Result<()> {
    // 1. 加载配置
    let config = Config::load()?;
    tracing::info!(project_id = %config.gitlab.project_id, "配置已加载");

    // 2. 创建 GitLab 客户端
    let gitlab: Arc<dyn GitlabApi> = Arc::new(GitlabClient::new(
        &config.gitlab.url,
        &config.gitlab.token,
    ));
    let git_ops = GitOps::new(std::env::current_dir()?);

    // 3. dry_run 模式: 打印配置后退出
    if dry_run {
        println!("=== devnpc dry-run 模式 ===");
        println!("LLM: {} {}", config.llm.base_url, config.llm.model);
        println!("GitLab: {} project={}", config.gitlab.url, config.gitlab.project_id);
        println!("Limits: max_iterations={}, max_ci_retries={}", config.limits.max_iterations, config.limits.max_ci_retries);
        println!("Branch prefix: {}", config.project.branch_prefix);
        if let Some(t) = task {
            println!("Task: {t}");
        }
        return Ok(());
    }

    // 3.5 初始化 MCP Gateway
    let mcp_gateway = if config.mcp.enabled {
        let gateway = devnpc::adapter::mcp_gateway::McpGateway::new(config.mcp.clone());
        // 启动 codemap (硬编码的内置服务器)
        if let Err(e) = gateway.start_codemap().await {
            tracing::warn!(error = %e, "codemap 启动失败");
        }
        // 从 YAML 配置加载 MCP 服务器 (npc-config/mcp-servers/*.yml)
        if config.npc_config.enabled {
            let mcp_dir = std::path::Path::new(&config.npc_config.base_dir).join("mcp-servers");
            if let Err(e) = gateway.load_from_yaml(&mcp_dir).await {
                tracing::warn!(error = %e, "从 YAML 加载 MCP 服务器失败");
            }
        }
        // 连接所有 MCP 服务器
        if let Err(e) = gateway.connect_all().await {
            tracing::warn!(error = %e, "MCP 服务器连接失败");
        }
        tracing::info!("MCP Gateway 已启用");
        Some(gateway)
    } else {
        None
    };

    // 4. 解析触发源
    let trigger = parse_trigger(task, &*gitlab, config.gitlab.project_id, &config.trigger).await?;
    tracing::info!(?trigger, "触发源解析结果");

    // 5-8. 根据触发类型执行
    let (task_spec, mr_iid, issue_iid) = match trigger {
        Trigger::Manual { task } => (task, None, None),
        Trigger::MrTask { mr_iid, task } => (task, Some(mr_iid), None),
        Trigger::IssueTask { issue_iid, task } => (task, None, Some(issue_iid)),
        Trigger::None => {
            tracing::info!("未检测到有效触发,退出");
            println!("devnpc: 未检测到 @devnpc 提及或 --task 参数");
            return Ok(());
        }
    };

    let issue_iid = issue_iid.or(task_spec.target_issue);

    let context = if let Some(iid) = issue_iid {
        tracing::info!(issue_iid = iid, "构建上下文");
        Some(Context::build(&*gitlab, &git_ops, config.gitlab.project_id, iid, &config.summary, &config.context, &config.project).await?)
    } else {
        tracing::warn!("无 Issue IID,跳过上下文构建");
        None
    };

    // 6. 创建 Orchestrator 并执行
    let start_time = chrono::Utc::now();

    // 创建模型路由 (simple/complex 两种)
    let simple_model = create_simple_model(&config)?;
    let complex_model = create_complex_model(&config)?;

    // 按任务复杂度为主 Agent 选择模型 (模型路由真正落地)
    let task_complexity = devnpc::adapter::orchestrator::classify_task_complexity(
        &task_spec.description,
    );
    let model = match task_complexity {
        devnpc::adapter::orchestrator::TaskComplexity::Simple => {
            tracing::info!(?task_complexity, "主 Agent 使用 simple_model");
            simple_model.clone()
        }
        devnpc::adapter::orchestrator::TaskComplexity::Complex => {
            tracing::info!(?task_complexity, "主 Agent 使用 complex_model");
            complex_model.clone()
        }
    };

    // 收集 MCP 工具集
    let mcp_toolsets = if let Some(ref gateway) = mcp_gateway {
        let toolsets = gateway.take_toolsets().await;
        tracing::info!(count = toolsets.len(), "MCP 工具集已就绪");
        toolsets
    } else {
        Vec::new()
    };

    // 创建工具 (保存副本供子 Agent 使用)
    let tools = create_all_tools(
        &config,
        Some(gitlab.clone()),
        Some(config.gitlab.project_id),
        Vec::new(), // MCP 工具通过 toolset 添加到 Agent
    )?;
    let sub_tools = tools.clone();

    // 创建回调 (注入 SOP 配置 + 工具白名单)
    let callbacks = DevnpcCallbacks::new(
        config.project.sop_mode,
        config.project.forbidden_paths.clone(),
        config.tools.allowed_tools.clone(),
    );

    // 构建主 Agent
    let agent = LlmAgentBuilder::new("devnpc")
        .instruction(config.project.main_instruction.clone())
        .model(model.clone())
        .before_tool_callback(callbacks.before_tool_callback())
        .after_model_callback(callbacks.after_model_callback());

    let agent = tools.into_iter().fold(agent, |builder, tool| {
        builder.tool(tool)
    });

    let agent = mcp_toolsets.into_iter().fold(agent, |builder, toolset| {
        builder.toolset(toolset)
    });

    let agent = agent.build().map_err(|e| {
        devnpc::error::DevnpcError::Config(format!("Agent 构建失败: {e}"))
    })?;

    // 初始化长期记忆
    let memory_store = if config.memory.enabled {
        let store = devnpc::adapter::memory::MemoryStore::new(
            config.memory.clone(),
            config.memory.max_search_results,
        );
        if let Err(e) = store.initialize() {
            tracing::warn!(error = %e, "记忆存储初始化失败");
        }
        tracing::info!("长期记忆系统已启用");
        Some(store)
    } else {
        None
    };

    // 加载 npc-config (角色 + SOP),失败时降级为硬编码默认指令
    let npc_config = if config.npc_config.enabled {
        let config_dir = std::path::Path::new(&config.npc_config.base_dir);
        match NpcConfig::load(config_dir) {
            Ok(c) => {
                tracing::info!(
                    roles = c.roles.len(),
                    sops = c.sops.len(),
                    teams = c.teams.len(),
                    "npc-config 已加载,子 Agent 将使用 YAML 角色配置"
                );
                Some(c)
            }
            Err(e) => {
                tracing::warn!(error = %e, "npc-config 加载失败,子 Agent 降级使用默认指令");
                None
            }
        }
    } else {
        tracing::info!("npc-config 已禁用,子 Agent 使用默认指令");
        None
    };

    // 角色映射:
    // - code_agent → developer + feature sop (功能开发)
    // - fix_agent → developer + bugfix sop (Bug 修复)
    // - review_agent → tester + test-gen sop (测试验证)
    let (code_role, code_sop) = npc_config
        .as_ref()
        .and_then(|c| c.role_with_default_sop("developer"))
        .map(|(r, s)| {
            let sop = npc_config.as_ref().and_then(|c| c.sop("feature")).or(s);
            (Some(r), sop)
        })
        .unwrap_or((None, None));

    let (fix_role, fix_sop) = npc_config
        .as_ref()
        .and_then(|c| c.role_with_default_sop("developer"))
        .map(|(r, s)| (Some(r), s))
        .unwrap_or((None, None));

    let (review_role, review_sop) = npc_config
        .as_ref()
        .and_then(|c| c.role_with_default_sop("tester"))
        .map(|(r, s)| (Some(r), s))
        .unwrap_or((None, None));

    // PM 角色: pm + requirement-decompose sop (用于 Team 编排的需求分解阶段)
    let (pm_role, pm_sop) = npc_config
        .as_ref()
        .and_then(|c| c.role_with_default_sop("pm"))
        .map(|(r, s)| {
            let sop = npc_config.as_ref().and_then(|c| c.sop("requirement-decompose")).or(s);
            (Some(r), sop)
        })
        .unwrap_or((None, None));

    // Skill 匹配: 根据任务类型和描述自动匹配领域专家技能
    // - Fix 任务 → 匹配 fix 类型的 skills (如 security)
    // - Review 任务 → 匹配 review 类型的 skills (如 security)
    // - Implement/Refactor → 匹配 implement 类型的 skills (如 frontend/backend/database)
    let matched_skills: Vec<&devnpc::config::skill::Skill> = npc_config
        .as_ref()
        .map(|c| c.skills.match_skills(&task_spec.kind, &task_spec.description))
        .unwrap_or_default();

    if !matched_skills.is_empty() {
        let names: Vec<&str> = matched_skills.iter().map(|s| s.name.as_str()).collect();
        tracing::info!(skills = ?names, "匹配到领域技能,将注入子 Agent");
    }

    // 为不同子 Agent 选择合适的 skills:
    // - code_agent: implement/refactor 类型技能 (frontend/backend/database)
    // - fix_agent:  fix 类型技能 (security)
    // - review_agent: review 类型技能 (security)
    let code_skills: Vec<&devnpc::config::skill::Skill> = matched_skills
        .iter()
        .filter(|s| {
            s.scenarios.task_kinds.iter().any(|k| k == "implement" || k == "refactor")
        })
        .copied()
        .collect();
    let fix_skills: Vec<&devnpc::config::skill::Skill> = matched_skills
        .iter()
        .filter(|s| s.scenarios.task_kinds.iter().any(|k| k == "fix"))
        .copied()
        .collect();
    let review_skills: Vec<&devnpc::config::skill::Skill> = matched_skills
        .iter()
        .filter(|s| s.scenarios.task_kinds.iter().any(|k| k == "review"))
        .copied()
        .collect();

    // 构建子 Agent (按职责分配模型 + 角色/SOP/Skill 三层注入)
    // - Code Agent: 代码修改 → complex_model + developer role + feature sop + implement skills
    // - Fix Agent:  CI 修复 → complex_model + developer role + bugfix sop + fix skills
    // - Review Agent: 代码审查 → simple_model + tester role + test-gen sop + review skills
    let code_agent = match build_code_agent(
        sub_tools.clone(),
        complex_model.clone(),
        code_role,
        code_sop,
        &code_skills,
    ) {
        Ok(agent) => Some(Arc::new(agent)),
        Err(e) => {
            tracing::warn!(error = %e, "Code Agent 构建失败,降级运行");
            None
        }
    };
    let fix_agent = match build_fix_agent(
        sub_tools.clone(),
        complex_model.clone(),
        fix_role,
        fix_sop,
        &fix_skills,
    ) {
        Ok(agent) => Some(Arc::new(agent)),
        Err(e) => {
            tracing::warn!(error = %e, "Fix Agent 构建失败,降级运行");
            None
        }
    };
    let review_agent = match build_review_agent(
        sub_tools.clone(),
        simple_model.clone(),
        review_role,
        review_sop,
        &review_skills,
    ) {
        Ok(agent) => Some(Arc::new(agent)),
        Err(e) => {
            tracing::warn!(error = %e, "Review Agent 构建失败,降级运行");
            None
        }
    };

    // PM Agent: 需求分解 → simple_model + pm role + requirement-decompose sop
    // 用于 Team 编排 (feature-team: PM→Developer→Tester) 的需求拆分阶段
    let pm_agent = match build_pm_agent(
        sub_tools.clone(),
        simple_model.clone(),
        pm_role,
        pm_sop,
        &[], // PM 不注入领域 skills,保持通用需求分解
    ) {
        Ok(agent) => Some(Arc::new(agent)),
        Err(e) => {
            tracing::warn!(error = %e, "PM Agent 构建失败,Team 编排降级为单 Agent 模式");
            None
        }
    };

    // 创建 Orchestrator
    let mut orchestrator = devnpc::adapter::orchestrator::Orchestrator::new(
        Arc::new(agent),
        code_agent.clone(),
        fix_agent,
        review_agent.clone(),
        Some(simple_model),
        Some(complex_model),
        memory_store,
    );

    // 注册 Team 编排用角色 Agent (PM→Developer→Tester 协作流程)
    // 仅当 feature-team 配置存在且 PM Agent 构建成功时启用
    let team_available = npc_config
        .as_ref()
        .and_then(|c| c.team("feature-team"))
        .is_some()
        && pm_agent.is_some()
        && code_agent.is_some()
        && review_agent.is_some();

    if team_available {
        if let Some(ref pm) = pm_agent {
            orchestrator.register_team_agent("pm", pm.clone());
        }
        if let Some(ref code) = code_agent {
            orchestrator.register_team_agent("developer", code.clone());
        }
        if let Some(ref review) = review_agent {
            orchestrator.register_team_agent("tester", review.clone());
        }
        tracing::info!("Team 编排已就绪: feature-team (PM→Developer→Tester 协作流程)");
    }

    let orchestrator = Arc::new(orchestrator);

    // 创建 SessionService 和初始状态
    let (session_service, session_id) = create_session_service();
    let initial_state = context.as_ref().map(build_initial_state).unwrap_or_default();

    // 执行 Agent
    // - Team 模式 (Complex + Implement 任务 + feature-team 配置就绪): PM→Developer→Tester 协作流程
    // - 单 Agent 模式 (Simple 任务 或 Fix/Review/其他): 主 Agent 直接执行 (带记忆注入)
    //
    // 简单任务 (read/summarize/explain 等) 即使 TaskKind=Implement 也跳过 Team,
    // 避免 "读取+总结" 类只读任务被路由到完整 Team 编排造成资源浪费与误改代码。
    let use_team_mode = team_available
        && matches!(task_spec.kind, devnpc::trigger::parser::TaskKind::Implement)
        && matches!(task_complexity, devnpc::adapter::orchestrator::TaskComplexity::Complex);

    // Team 协作步骤 (仅在 Team 模式下填充,用于报告可视化)
    let mut team_steps_for_report: Vec<devnpc::report::collector::TeamStepSummary> = Vec::new();

    let final_text = if use_team_mode {
        let team = npc_config
            .as_ref()
            .and_then(|c| c.team("feature-team"))
            .expect("team_available 隐含 feature-team 配置存在");
        tracing::info!("启用 Team 编排模式: PM→Developer→Tester 协作流程");
        let team_result = orchestrator
            .run_team(team, &task_spec.description)
            .await?;
        // 收集 Team 步骤供报告渲染
        team_steps_for_report = team_result
            .steps
            .iter()
            .map(|s| devnpc::report::collector::TeamStepSummary {
                role: s.role.clone(),
                instruction: s.instruction.clone(),
                output: s.output.clone(),
                signals: s.signals.clone(),
            })
            .collect();
        if team_result.summary.is_empty() {
            task_spec.description.clone()
        } else {
            team_result.summary
        }
    } else {
        // 使用 run_with_memory 自动检索相关历史记忆并注入到 Agent 上下文
        orchestrator
            .run_with_memory(&task_spec.description, session_service, &session_id, initial_state)
            .await?
    };

    let end_time = chrono::Utc::now();
    let summary = if final_text.is_empty() {
        task_spec.description.clone()
    } else {
        final_text
    };

    tracing::info!(summary = %summary, "Agent 执行完成");

    // 7. CI 控制器: 创建 MR → 轮询 pipeline → 修复
    let branch = format!("{}/{}", config.project.branch_prefix, chrono::Utc::now().format("%Y%m%d%H%M%S"));
    let ci_outcome = run_ci_controller(
        &*gitlab,
        &config,
        &branch,
        &summary,
        mr_iid,
        &context,
        orchestrator.clone(),
    )
    .await?;

    // 7.5 持久化任务记录到长期记忆 (失败不阻断主流程)
    if let Some(ref store) = orchestrator.memory_store {
        let usage_stats = orchestrator.take_usage_stats();
        let duration_secs = (end_time - start_time).num_seconds().max(0) as u64;
        let success = matches!(ci_outcome, CiOutcome::Passed { .. });

        // 优先从 GitLab MR changes 接口获取精确的修改文件列表 (用于记忆检索),
        // 失败或非 MR 场景回退到旧的占位表示。
        let modified_files = fetch_ci_modified_files(&*gitlab, config.gitlab.project_id, &ci_outcome).await;

        let record = devnpc::adapter::memory::TaskRecord {
            task_description: task_spec.description.clone(),
            result_summary: summary.clone(),
            modified_files,
            duration_secs,
            token_consumption: usage_stats.total_tokens().max(0) as u64,
            success,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(e) = store.save_task_record(record) {
            tracing::warn!(error = %e, "任务记录持久化失败 (不阻断主流程)");
        } else {
            tracing::info!("任务记录已持久化到长期记忆");
        }

        // CI 失败经验持久化 (Failed/Timeout/Error 时记录,便于后续同类问题检索)
        let (should_save_exp, failure_type, error_msg) = match &ci_outcome {
            CiOutcome::Failed { last_error, .. } => {
                (true, "CI失败".to_string(), last_error.clone())
            }
            CiOutcome::Timeout { stage, .. } => {
                (true, "CI超时".to_string(), format!("阶段: {stage}"))
            }
            CiOutcome::Error { reason, .. } => {
                (true, "CI异常".to_string(), reason.clone())
            }
            CiOutcome::Passed { .. } => (false, String::new(), String::new()),
        };
        if should_save_exp {
            let exp = devnpc::adapter::memory::FixExperience {
                failure_type,
                error_message: error_msg,
                root_cause: task_spec.description.clone(),
                fix_method: summary.clone(),
                success,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            if let Err(e) = store.save_fix_experience(exp) {
                tracing::warn!(error = %e, "修复经验持久化失败 (不阻断主流程)");
            } else {
                tracing::info!("修复经验已持久化到长期记忆");
            }
        }
    }

    // 8. 生成报告并发布
    let trajectory = Trajectory::new();
    // 取出 Orchestrator 累积的真实 token 使用统计
    let usage_stats = orchestrator.take_usage_stats();
    let report_data = build_report(
        &trajectory,
        &summary,
        &ci_outcome,
        &task_spec.description,
        start_time,
        end_time,
        &usage_stats,
        &team_steps_for_report,
        &config.cost,
    );
    let html = devnpc::report::html::generate_html(&report_data);
    let report_url = publisher::publish(&html, &config.report.target, &config.report).await?;
    tracing::info!(report_url = %report_url, "报告已发布");

    // 9. 评论 MR 或输出结果
    let summary_text = format!(
        "## devnpc 执行报告\n\n**状态**: {}\n**摘要**: {}\n\n**分支**: {}\n**报告**: {}",
        report_data.status,
        summary,
        branch,
        report_url,
    );
    println!("{}", summary_text);

    if let Some(mr_iid) = mr_iid {
        gitlab
            .create_mr_note(config.gitlab.project_id, mr_iid, &summary_text)
            .await?;
        tracing::info!(mr_iid = mr_iid, "评论已发布到 MR");
    }

    Ok(())
}

/// 解析触发源: 优先 --task,其次 CI 环境变量
async fn parse_trigger(
    task: Option<&str>,
    gitlab: &dyn GitlabApi,
    project_id: u64,
    trigger_config: &devnpc::config::TriggerConfig,
) -> Result<Trigger> {
    if let Some(task_str) = task {
        // 手动触发
        let task_spec = devnpc::trigger::parser::TaskSpec {
            kind: classify_task(task_str),
            description: task_str.to_string(),
            target_issue: None,
            acceptance_criteria: vec![],
        };
        return Ok(Trigger::Manual { task: task_spec });
    }

    // CI 环境变量: MR 评论
    if let Ok(mr_iid_str) = std::env::var(&trigger_config.ci_mr_iid_var) {
        let mr_iid: u64 = mr_iid_str.parse().map_err(|e| {
            devnpc::error::DevnpcError::Config(format!(
                "{} 不是有效数字: {mr_iid_str}, {e}",
                trigger_config.ci_mr_iid_var
            ))
        })?;
        let notes = gitlab.get_mr_notes(project_id, mr_iid).await?;
        // 找到最新 @devnpc 提及
        for note in notes.iter().rev() {
            if let Some(task_spec) = parse_mention_with_pattern(&note.body, &trigger_config.mention_regex) {
                return Ok(Trigger::MrTask { mr_iid, task: task_spec });
            }
        }
        tracing::info!(mr_iid = mr_iid, "MR 中未发现 @devnpc 提及");
    }

    // CI 环境变量: Issue 评论
    if let Ok(issue_iid_str) = std::env::var(&trigger_config.ci_issue_iid_var) {
        let issue_iid: u64 = issue_iid_str.parse().map_err(|e| {
            devnpc::error::DevnpcError::Config(format!(
                "{} 不是有效数字: {issue_iid_str}, {e}",
                trigger_config.ci_issue_iid_var
            ))
        })?;
        let notes = gitlab.get_issue_notes(project_id, issue_iid).await?;
        for note in notes.iter().rev() {
            if let Some(task_spec) = parse_mention_with_pattern(&note.body, &trigger_config.mention_regex) {
                return Ok(Trigger::IssueTask { issue_iid, task: task_spec });
            }
        }
    }

    Ok(Trigger::None)
}

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

/// 运行 CI 控制器: 创建 MR → 轮询 pipeline → 修复
async fn run_ci_controller(
    gitlab: &dyn GitlabApi,
    config: &Config,
    branch: &str,
    summary: &str,
    _mr_iid: Option<u64>,
    _context: &Option<Context>,
    orchestrator: Arc<devnpc::adapter::orchestrator::Orchestrator>,
) -> Result<CiOutcome> {
    // 创建 MR (如果当前没有关联 MR)
    // title 截断到 GitLab 限制内 (255 字符),优先使用首行非空文本
    let mr_title = {
        let prefix = "devnpc: ";
        let max_len = 255usize.saturating_sub(prefix.len());
        // 提取首行非空文本作为标题
        let first_line = summary
            .lines()
            .map(|l| l.trim())
            .find(|l| !l.is_empty())
            .unwrap_or(summary.trim());
        // 去除可能的 markdown 标记
        let clean = first_line.trim_start_matches('#').trim();
        let clean = if clean.is_empty() { first_line } else { clean };
        // 截断到最大长度
        let truncated = if clean.chars().count() > max_len {
            let chars: Vec<char> = clean.chars().take(max_len - 3).collect();
            format!("{}...", chars.into_iter().collect::<String>())
        } else {
            clean.to_string()
        };
        format!("{prefix}{truncated}")
    };
    let create_req = devnpc::gitlab_api::CreateMrReq {
        source_branch: branch.to_string(),
        target_branch: config.project.target_branch.clone(),
        title: mr_title,
        description: format!("由 devnpc 自动创建的 MR\n\n## 摘要\n{}", summary),
        draft: true,
    };

    match gitlab.create_mr(config.gitlab.project_id, create_req).await {
        Ok(mr) => {
            tracing::info!(mr_iid = mr.iid, mr_url = %mr.web_url, "MR 已创建");

            // 使用 CiController 运行 CI 闭环
            let ci_config = config.ci.clone();
            let git_ops = GitOps::new(std::env::current_dir()?);
            let controller = CiController::new(
                ci_config,
                Box::new(GitlabClient::new(&config.gitlab.url, &config.gitlab.token)),
                git_ops,
                config.gitlab.project_id,
                Box::new(FixHandlerImpl {
                    orchestrator: orchestrator.clone(),
                }),
            );
            match controller.run(mr.iid, branch).await {
                Ok(outcome) => {
                    tracing::info!(?outcome, "CI 闭环完成");
                    Ok(outcome)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "CI 控制器运行失败");
                    Ok(CiOutcome::Error {
                        mr_iid: mr.iid,
                        reason: format!("CI 控制器运行失败: {e}"),
                    })
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "创建 MR 失败,可能已存在");
            Ok(CiOutcome::Error {
                mr_iid: 0,
                reason: format!("创建 MR 失败: {e}"),
            })
        }
    }
}

/// 从 CI 结果中提取修改的文件列表
///
fn ci_outcome_mr_iid(ci_outcome: &CiOutcome) -> u64 {
    match ci_outcome {
        CiOutcome::Passed { mr_iid, .. }
        | CiOutcome::Failed { mr_iid, .. }
        | CiOutcome::Timeout { mr_iid, .. }
        | CiOutcome::Error { mr_iid, .. } => *mr_iid,
    }
}

/// 获取 CI 闭环实际修改的文件列表 (用于长期记忆检索)。
/// 优先通过 GitLab MR changes 接口获取精确的 diff 文件列表;失败或非 MR
/// 场景回退到基于 CI 状态的占位表示 (Passed → "MR !{iid}",其他 → 空)。
async fn fetch_ci_modified_files(
    gitlab: &dyn GitlabApi,
    project_id: u64,
    ci_outcome: &CiOutcome,
) -> Vec<String> {
    let mr_iid = ci_outcome_mr_iid(ci_outcome);
    if mr_iid > 0 {
        match gitlab
            .get_mr_changes(project_id, mr_iid)
            .await
        {
            Ok(changes) if !changes.is_empty() => {
                // renamed 用 new_path, deleted 用 old_path,其余用 new_path
                let files: Vec<String> = changes
                    .iter()
                    .map(|c| {
                        if c.deleted_file {
                            c.old_path.clone()
                        } else {
                            c.new_path.clone()
                        }
                    })
                    .collect();
                tracing::info!(count = files.len(), "从 GitLab MR changes 获取到修改文件列表");
                return files;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "获取 MR changes 失败,回退到占位表示");
            }
        }
    }
    if mr_iid > 0 {
        vec![format!("MR !{mr_iid}")]
    } else {
        Vec::new()
    }
}

/// 构建报告数据
#[allow(clippy::too_many_arguments)]
fn build_report(
    trajectory: &Trajectory,
    summary: &str,
    ci_outcome: &CiOutcome,
    task_description: &str,
    start_time: chrono::DateTime<chrono::Utc>,
    end_time: chrono::DateTime<chrono::Utc>,
    usage_stats: &devnpc::adapter::orchestrator::UsageStats,
    team_steps: &[devnpc::report::collector::TeamStepSummary],
    cost_config: &devnpc::config::CostConfig,
) -> ReportData {
    let llm_calls = trajectory
        .events
        .iter()
        .filter(|e| matches!(e, TrajectoryEvent::LlmCall { .. }))
        .count() as u32;
    let tool_calls = trajectory
        .events
        .iter()
        .filter(|e| matches!(e, TrajectoryEvent::ToolCall { .. }))
        .count() as u32;

    let (status, mr_iid, pipeline_id, ci_retries, mr_url, ci_url) = match ci_outcome {
        CiOutcome::Passed {
            mr_iid,
            pipeline_id,
            attempts,
        } => (
            "passed".into(),
            Some(*mr_iid),
            Some(*pipeline_id),
            *attempts,
            None,
            Some(format!("pipeline #{pipeline_id}")),
        ),
        CiOutcome::Failed {
            mr_iid,
            last_error,
            attempts,
        } => (
            format!("failed: {last_error}"),
            Some(*mr_iid),
            None,
            *attempts,
            None,
            None,
        ),
        CiOutcome::Timeout { mr_iid, stage } => (
            format!("timeout: {stage}"),
            Some(*mr_iid),
            None,
            0,
            None,
            None,
        ),
        CiOutcome::Error { mr_iid, reason } => (
            format!("error: {reason}"),
            Some(*mr_iid),
            None,
            0,
            None,
            None,
        ),
    };

    let duration_secs = (end_time - start_time).num_seconds().max(0) as u64;
    // 优先使用 Orchestrator 累积的真实 token 数据; 若 stats 为零 (无 usage_metadata), 回退到 trajectory 估算
    let (input_tokens, output_tokens, estimated_cost_usd, effective_llm_calls) =
        if usage_stats.llm_calls > 0 || usage_stats.total_tokens() > 0 {
            // 真实 token 数据 (provider 返回的 usage_metadata)
            let cost = usage_stats.estimated_cost_or_default();
            tracing::info!(
                input_tokens = usage_stats.input_tokens,
                output_tokens = usage_stats.output_tokens,
                llm_calls = usage_stats.llm_calls,
                cost_usd = cost,
                "使用真实 provider usage_metadata 生成报告"
            );
            (
                usage_stats.input_tokens as u64,
                usage_stats.output_tokens as u64,
                cost,
                usage_stats.llm_calls,
            )
        } else {
            // 回退: trajectory 事件计数 * 平均 token 估算 (兼容 provider 未返回 usage 的场景)
            tracing::warn!("Orchestrator 未累积到 usage_metadata, 回退到固定估算");
            let in_tok = llm_calls as u64 * cost_config.est_input_tokens_per_call;
            let out_tok = llm_calls as u64 * cost_config.est_output_tokens_per_call;
            let cost = devnpc::adapter::orchestrator::UsageStats::estimate_cost_with_rates(
                in_tok as i64,
                out_tok as i64,
                cost_config.input_rate,
                cost_config.output_rate,
            );
            (in_tok, out_tok, cost, llm_calls as u64)
        };

    ReportData {
        status,
        duration_secs,
        token_total: input_tokens + output_tokens,
        llm_calls: effective_llm_calls as u32,
        tool_calls,
        ci_retries,
        mr_url,
        ci_url,
        summary: summary.to_string(),
        task_description: task_description.to_string(),
        trajectory: TrajectorySummary::default(),
        cost_estimate: CostEstimate {
            input_tokens,
            output_tokens,
            estimated_cost_usd,
        },
        mr_iid,
        pipeline_id,
        started_at: start_time.to_rfc3339(),
        finished_at: end_time.to_rfc3339(),
        team_steps: team_steps.to_vec(),
    }
}

fn print_config() -> Result<()> {
    match Config::load() {
        Ok(config) => {
            println!("=== devnpc 配置 ===");
            println!("LLM:");
            println!("  base_url: {}", config.llm.base_url);
            println!("  model: {}", config.llm.model);
            println!(
                "  api_key: <configured>",
            );
            println!("GitLab:");
            println!("  url: {}", config.gitlab.url);
            println!("  project_id: {}", config.gitlab.project_id);
            println!("Limits:");
            println!("  max_iterations: {}", config.limits.max_iterations);
            println!("  max_ci_retries: {}", config.limits.max_ci_retries);
            println!("Project:");
            println!("  sop_mode: {:?}", config.project.sop_mode);
            println!("  branch_prefix: {}", config.project.branch_prefix);
            println!(
                "  forbidden_paths: {:?}",
                config.project.forbidden_paths
            );
            println!(
                "  required_checks: {:?}",
                config.project.required_checks
            );
            println!(
                "  guidelines_markdown_len: {}",
                config.project.guidelines_markdown.len()
            );
            println!("Report:");
            println!("  target: {:?}", config.report.target);
            println!("Model Routing:");
            println!(
                "  simple_model: {}",
                config.model_routing.simple_model
            );
            println!(
                "  complex_model: {}",
                config.model_routing.complex_model
            );
            Ok(())
        }
        Err(e) => {
            eprintln!("配置加载失败: {e}");
            Err(e)
        }
    }
}

fn print_info() {
    println!("devnpc {}", env!("CARGO_PKG_VERSION"));
    println!("基于 GitLab 的企业级研发流程 AI 智能体");
    println!();
    println!("基于 adk-rust 框架: LlmAgent + Runner + FunctionTool");
}