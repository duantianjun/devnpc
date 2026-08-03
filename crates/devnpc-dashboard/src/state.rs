//! 应用共享状态

use crate::realtime::RealtimeHub;
use crate::storage::queries::Storage;

/// axum 共享状态,通过 State<AppState> 注入 handler 与中间件
#[derive(Clone)]
pub struct AppState {
    /// SQLite 存储层
    pub storage: Storage,
    /// 实时事件中心
    pub hub: std::sync::Arc<RealtimeHub>,
    /// 推送鉴权 token (空字符串表示未配置)
    pub token: String,
}
