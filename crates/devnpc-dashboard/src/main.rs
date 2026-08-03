//! devnpc-dashboard CLI 入口
//!
//! Task 11 中填充完整启动流程。

use clap::Parser;

#[derive(Parser)]
#[command(name = "devnpc-dashboard", about = "devnpc 可观测 Dashboard 服务")]
struct Cli {
    /// 监听端口 (默认 8080)
    #[arg(long, env = "DEVNPC_DASHBOARD_PORT")]
    port: Option<u16>,

    /// 监听地址 (默认 0.0.0.0)
    #[arg(long, env = "DEVNPC_DASHBOARD_HOST")]
    host: Option<String>,

    /// SQLite 数据库路径 (默认 ./devnpc-dashboard.db)
    #[arg(long, env = "DEVNPC_DASHBOARD_DB")]
    db: Option<String>,

    /// 推送鉴权 token
    #[arg(long, env = "DEVNPC_DASHBOARD_TOKEN")]
    token: Option<String>,

    /// 实时环形缓冲容量 (默认 1000)
    #[arg(long, env = "DEVNPC_DASHBOARD_REALTIME_BUFFER")]
    realtime_buffer: Option<usize>,
}

fn main() {
    let _cli = Cli::parse();
    eprintln!("devnpc-dashboard: 服务启动逻辑在 Task 11 实现");
}
