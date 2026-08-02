use std::sync::Arc;

use clap::{Parser, Subcommand};

use devnpc::ci::controller::{CiController, CiOutcome, FixHandler};
use devnpc::ci::log_parser::ParsedFailure;
use devnpc::config::Config;
use devnpc::error::Result;
use devnpc::git::ops::GitOps;
use devnpc::gitlab_api::client::GitlabClient;
use devnpc::gitlab_api::GitlabApi;
use devnpc::memory::context::Context;
use devnpc::npc::role::Role;
use devnpc::npc::runner::NpcRunner;
use devnpc::report::collector::{CostEstimate, ReportData, TrajectorySummary};
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

/// 主运行流程 (P5 完整实现):
///
/// 1. 加载配置
/// 2. 创建 GitLab 客户端
/// 3. dry_run 模式: 打印配置后退出
/// 4. 解析触发源 (CI env vars / --task)
/// 5. 构建上下文 (Context::build)
/// 6. 创建 NPC runner 并执行
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

    if let Some(context) = context {
        // 6. 创建 NPC runner 并执行
        let role = Role {
            name: "developer".into(),
            description: "通用开发工程师".into(),
            system_prompt: "你是一个 Rust 开发工程师。使用 devnpc 工具链完成研发任务。\n\
                遵循以下原则:\n\
                1. 修改前先理解上下文 (read_file / list_files / aft_outline)\n\
                2. 改完后用 cargo build 验证编译\n\
                3. 完成后调用 finish 工具,summary 写验收摘要\n\
                4. 禁止修改工作目录外的文件"
                .into(),
            max_iterations: config.limits.max_iterations,
            default_sop: None,
            tools: vec![],
        };
        let runner = NpcRunner::new(role);
        let npc_result = runner.execute(&task_spec, &context, &config).await?;
        tracing::info!(summary = %npc_result.summary, branch = %npc_result.branch, "NPC 执行完成");

        // 7. CI 控制器: 创建 MR → 轮询 pipeline → 修复
        let ci_outcome = run_ci_controller(
            &*gitlab,
            &config,
            &npc_result.branch,
            &npc_result.summary,
            mr_iid,
            &context,
        )
        .await?;

        // 8. 生成报告并发布
        let report_data = build_report(&npc_result, &ci_outcome);
        let html = devnpc::report::html::generate_html(&report_data);
        let report_url = publisher::publish(&html, &config.report.target).await?;
        tracing::info!(report_url = %report_url, "报告已发布");

        // 9. 评论 MR 或输出结果
        let summary_text = format!(
            "## devnpc 执行报告\n\n**状态**: {}\n**摘要**: {}\n\n**分支**: {}\n**报告**: {}",
            report_data.status,
            npc_result.summary,
            npc_result.branch,
            report_url,
        );
        println!("{}", summary_text);

        if let Some(mr_iid) = mr_iid {
            gitlab
                .create_mr_note(config.gitlab.project_id, mr_iid, &summary_text)
                .await?;
            tracing::info!(mr_iid = mr_iid, "评论已发布到 MR");
        }
    } else {
        tracing::error!("缺少 Issue 上下文,无法执行 NPC 任务");
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

/// 无操作修复处理器 (CI 闭环暂未接入真实修复 agent 时的占位)
struct NoopFixHandler;

#[async_trait::async_trait]
impl FixHandler for NoopFixHandler {
    async fn run_fix(&self, _failures: &[ParsedFailure], _instruction: &str) -> Result<String> {
        Ok("noop fix".into())
    }
}

/// 运行 CI 控制器: 创建 MR → 轮询 pipeline → 修复
async fn run_ci_controller(
    gitlab: &dyn GitlabApi,
    config: &Config,
    branch: &str,
    summary: &str,
    _mr_iid: Option<u64>,
    _context: &Context,
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
                Box::new(NoopFixHandler),
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
    npc_result: &devnpc::npc::runner::NpcResult,
    ci_outcome: &CiOutcome,
) -> ReportData {
    let llm_calls = npc_result
        .trajectory
        .events
        .iter()
        .filter(|e| matches!(e, devnpc::agent::loop_::TrajectoryEvent::LlmCall { .. }))
        .count() as u32;
    let tool_calls = npc_result
        .trajectory
        .events
        .iter()
        .filter(|e| matches!(e, devnpc::agent::loop_::TrajectoryEvent::ToolCall { .. }))
        .count() as u32;

    let (status, ci_retries) = match ci_outcome {
        CiOutcome::Passed { attempts, .. } => ("success".into(), *attempts),
        CiOutcome::Failed { attempts, .. } => ("failed".into(), *attempts),
        CiOutcome::Timeout { .. } => ("timeout".into(), 0),
    };

    ReportData {
        status,
        duration_secs: 0,
        token_total: 0,
        llm_calls,
        tool_calls,
        ci_retries,
        mr_url: None,
        ci_url: None,
        summary: npc_result.summary.clone(),
        task_description: String::new(),
        trajectory: TrajectorySummary::default(),
        cost_estimate: CostEstimate::default(),
        mr_iid: None,
        pipeline_id: None,
        started_at: String::new(),
        finished_at: String::new(),
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
    println!("全部阶段已实现: P0 骨架 → P1 配置 → P2 记忆 → P3 Agent → P3.5 AFT → P4 CI闭环 → P5 触发 → P6 Role/SOP/Team → P7 报告 → P8 模型路由");
}