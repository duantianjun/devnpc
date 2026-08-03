//! devnpc-dashboard CLI 入口
//!
//! 加载配置 -> 打开 SQLite -> 初始化 RealtimeHub -> 启动 axum 服务

use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use devnpc_dashboard::realtime::RealtimeHub;
use devnpc_dashboard::server::build_router;
use devnpc_dashboard::state::AppState;
use devnpc_dashboard::storage::queries::Storage;

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 加载 .env (可选,文件不存在不报错)
    let _ = dotenvy::dotenv();

    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cli = Cli::parse();
    let port = cli.port.unwrap_or(8080);
    let host = cli.host.unwrap_or_else(|| "0.0.0.0".into());
    let db_path = cli.db.unwrap_or_else(|| "./devnpc-dashboard.db".into());
    let token = cli.token.unwrap_or_default();
    let buffer_cap = cli.realtime_buffer.unwrap_or(1000);

    // 打开 SQLite (WAL + schema 迁移)
    let storage = Storage::open(&db_path)?;
    tracing::info!(db = %db_path, "SQLite 已就绪 (WAL 模式)");

    // 初始化 RealtimeHub
    let hub = RealtimeHub::new(buffer_cap);

    // 构建共享状态
    let state = AppState {
        storage,
        hub: Arc::new(hub),
        token,
    };

    // 构建路由
    let app = build_router(state);

    // 绑定监听
    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(addr = %addr, "devnpc-dashboard 服务已启动");

    axum::serve(listener, app).await?;

    Ok(())
}
