use clap::{Parser, Subcommand};

use devnpc::error::Result;

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

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run { task, dry_run }) => run(task.as_deref(), dry_run),
        Some(Commands::Config) => print_config(),
        Some(Commands::Info) | None => {
            print_info();
            Ok(())
        }
    }
}

fn run(task: Option<&str>, dry_run: bool) -> Result<()> {
    tracing::info!(task = ?task, dry_run, "启动 devnpc (P0 骨架)");
    println!("devnpc P0 骨架 - run 命令");
    if dry_run {
        println!("dry_run 模式: 不执行实际任务");
    }
    println!("P1+ 将实现完整功能");
    Ok(())
}

fn print_config() -> Result<()> {
    match devnpc::config::Config::load() {
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
    println!("阶段: P0 骨架");
    println!("后续: P1 配置+API → P2 记忆 → P3 Agent → P4 CI闭环 → P5 触发");
}
