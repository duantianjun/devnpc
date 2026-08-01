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
    println!("devnpc P0 骨架 - 配置加载待 P1 实现");
    Ok(())
}

fn print_info() {
    println!("devnpc {}", env!("CARGO_PKG_VERSION"));
    println!("基于 GitLab 的企业级研发流程 AI 智能体");
    println!();
    println!("阶段: P0 骨架");
    println!("后续: P1 配置+API → P2 记忆 → P3 Agent → P4 CI闭环 → P5 触发");
}
