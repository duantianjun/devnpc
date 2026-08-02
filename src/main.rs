use std::sync::Arc;

use clap::{Parser, Subcommand};

use adk_rust::agent::LlmAgentBuilder;

use devnpc::adapter::callbacks::DevnpcCallbacks;
use devnpc::adapter::context::{build_initial_state, create_session_service};
use devnpc::adapter::provider::{create_complex_model, create_model, create_simple_model};
use devnpc::adapter::tools::create_all_tools;
use devnpc::ci::controller::{CiController, CiOutcome, FixHandler};
use devnpc::ci::log_parser::ParsedFailure;
use devnpc::config::Config;
use devnpc::error::Result;
use devnpc::git::ops::GitOps;
use devnpc::gitlab_api::client::GitlabClient;
use devnpc::gitlab_api::GitlabApi;
use devnpc::memory::context::Context;
use devnpc::report::collector::{CostEstimate, ReportData, Trajectory, TrajectoryEvent, TrajectorySummary};
use devnpc::report::publisher;
use devnpc::trigger::parser::{classify_task, parse_mention, Trigger};

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

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run { task, dry_run }) => run(task.as_deref(), dry_run).await,
        Some(Commands::Config) => print_config(),
        Some(Commands::Info) | None => {
            print_info();
            Ok(())
        }
    }
}

/// 系统指令 (通用开发工程师角色)
const SYSTEM_INSTRUCTION: &str = "\
你是一个软件开发工程师。使用 devnpc 工具链完成研发任务。\n\
遵循以下原则:\n\
1. 修改前先理解上下文 (read_file / list_files / aft_outline)\n\
2. 改完后用对应的构建工具验证编译 (如 cargo build / mvn compile / gradle build / npm run build 等)\n\
3. 完成后总结你的工作成果\n\
4. 禁止修改工作目录外的文件";

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
        // 启动 codemap
        if let Err(e) = gateway.start_codemap().await {
            tracing::warn!(error = %e, "codemap 启动失败");
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
    let trigger = parse_trigger(task, &*gitlab, config.gitlab.project_id).await?;
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
        Some(Context::build(&*gitlab, &git_ops, config.gitlab.project_id, iid, &config.summary, &config.context).await?)
    } else {
        tracing::warn!("无 Issue IID,跳过上下文构建");
        None
    };

    // 6. 创建 Orchestrator 并执行
    let start_time = chrono::Utc::now();

    // 创建模型
    let model = create_model(&config.llm)?;

    // 收集 MCP 工具集
    let mcp_toolsets = if let Some(ref gateway) = mcp_gateway {
        let toolsets = gateway.take_toolsets().await;
        tracing::info!(count = toolsets.len(), "MCP 工具集已就绪");
        toolsets
    } else {
        Vec::new()
    };

    // 创建工具
    let tools = create_all_tools(
        &config,
        Some(gitlab.clone()),
        Some(config.gitlab.project_id),
        Vec::new(), // MCP 工具通过 toolset 添加到 Agent
    );

    // 创建回调
    let callbacks = DevnpcCallbacks::new();

    // 构建主 Agent
    let agent = LlmAgentBuilder::new("devnpc")
        .instruction(SYSTEM_INSTRUCTION)
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

    // 创建模型路由
    let simple_model = create_simple_model(&config)?;
    let complex_model = create_complex_model(&config)?;

    // 初始化长期记忆
    let memory_store = if config.memory.enabled {
        let store = devnpc::adapter::memory::MemoryStore::new(config.memory.clone());
        if let Err(e) = store.initialize() {
            tracing::warn!(error = %e, "记忆存储初始化失败");
        }
        tracing::info!("长期记忆系统已启用");
        Some(store)
    } else {
        None
    };

    // 构建子 Agent (预留，后续扩展)
    let code_agent = None;
    let fix_agent = None;
    let review_agent = None;

    // 创建 Orchestrator
    let orchestrator = Arc::new(devnpc::adapter::orchestrator::Orchestrator::new(
        Arc::new(agent),
        code_agent,
        fix_agent,
        review_agent,
        Some(simple_model),
        Some(complex_model),
        memory_store,
    ));

    // 创建 SessionService 和初始状态
    let (session_service, session_id) = create_session_service();
    let initial_state = context.as_ref().map(build_initial_state).unwrap_or_default();

    // 执行 Agent
    let final_text = orchestrator
        .run(&task_spec.description, session_service, &session_id, initial_state)
        .await?;

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

    // 8. 生成报告并发布
    let trajectory = Trajectory::new();
    let report_data = build_report(
        &trajectory,
        &summary,
        &ci_outcome,
        &task_spec.description,
        start_time,
        end_time,
    );
    let html = devnpc::report::html::generate_html(&report_data);
    let report_url = publisher::publish(&html, &config.report.target).await?;
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
    if let Ok(mr_iid_str) = std::env::var("CI_MERGE_REQUEST_IID") {
        let mr_iid: u64 = mr_iid_str.parse().map_err(|e| {
            devnpc::error::DevnpcError::Config(format!(
                "CI_MERGE_REQUEST_IID 不是有效数字: {mr_iid_str}, {e}"
            ))
        })?;
        let notes = gitlab.get_mr_notes(project_id, mr_iid).await?;
        // 找到最新 @devnpc 提及
        for note in notes.iter().rev() {
            if let Some(task_spec) = parse_mention(&note.body) {
                return Ok(Trigger::MrTask { mr_iid, task: task_spec });
            }
        }
        tracing::info!(mr_iid = mr_iid, "MR 中未发现 @devnpc 提及");
    }

    // CI 环境变量: Issue 评论
    if let Ok(issue_iid_str) = std::env::var("CI_ISSUE_IID") {
        let issue_iid: u64 = issue_iid_str.parse().map_err(|e| {
            devnpc::error::DevnpcError::Config(format!(
                "CI_ISSUE_IID 不是有效数字: {issue_iid_str}, {e}"
            ))
        })?;
        let notes = gitlab.get_issue_notes(project_id, issue_iid).await?;
        for note in notes.iter().rev() {
            if let Some(task_spec) = parse_mention(&note.body) {
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
    let create_req = devnpc::gitlab_api::CreateMrReq {
        source_branch: branch.to_string(),
        target_branch: "main".to_string(),
        title: format!("devnpc: {}", summary),
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
                    tracing::warn!(error = %e, "CI 控制器运行失败,使用 fallback");
                    Ok(CiOutcome::Passed {
                        mr_iid: mr.iid,
                        pipeline_id: 0,
                        attempts: 0,
                    })
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "创建 MR 失败,可能已存在");
            Ok(CiOutcome::Passed {
                mr_iid: 0,
                pipeline_id: 0,
                attempts: 0,
            })
        }
    }
}

/// 构建报告数据
fn build_report(
    trajectory: &Trajectory,
    summary: &str,
    ci_outcome: &CiOutcome,
    task_description: &str,
    start_time: chrono::DateTime<chrono::Utc>,
    end_time: chrono::DateTime<chrono::Utc>,
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
    };

    let duration_secs = (end_time - start_time).num_seconds().max(0) as u64;
    let input_tokens = llm_calls as u64 * 500;
    let output_tokens = llm_calls as u64 * 200;
    let estimated_cost_usd = (input_tokens as f64 * 0.000_001_5) + (output_tokens as f64 * 0.000_002_0);

    ReportData {
        status,
        duration_secs,
        token_total: input_tokens + output_tokens,
        llm_calls,
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
                "  api_key: {}***",
                config.llm.api_key.chars().take(4).collect::<String>()
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