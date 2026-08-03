//! askama 模板对应的 Rust struct 定义
//!
//! 每个页面一个 struct,通过 `#[derive(Template)]` 关联 HTML 模板。
//! 模板文件位于 `src/views/`,通过 crate 根目录的 `askama.toml` 配置
//! (`dirs = ["src/views"]`)。子模板通过 `{% extends "layout.html" %}`
//! 继承公共布局,布局中通过 `active_nav` 字段高亮当前导航项。

use askama::Template;

use crate::storage::queries::TaskRow;

/// 任务列表页
#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    /// 当前激活的导航项标识: tasks/realtime/trends/cost/ci/sop
    pub active_nav: String,
}

/// 任务详情页
#[derive(Template)]
#[template(path = "task_detail.html")]
pub struct TaskDetailTemplate {
    pub active_nav: String,
    pub task: TaskRow,
}

/// 实时监控页
#[derive(Template)]
#[template(path = "realtime.html")]
pub struct RealtimeTemplate {
    pub active_nav: String,
}

/// 趋势统计页
#[derive(Template)]
#[template(path = "trends.html")]
pub struct TrendsTemplate {
    pub active_nav: String,
}

/// 成本分析页
#[derive(Template)]
#[template(path = "cost.html")]
pub struct CostTemplate {
    pub active_nav: String,
}
