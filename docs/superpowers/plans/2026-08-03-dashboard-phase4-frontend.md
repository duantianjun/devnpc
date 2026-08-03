# Dashboard Phase 4: 前端视图层（askama 模板 + LayUI + ECharts）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 devnpc-dashboard 实现 7 个服务端渲染页面（askama 模板 + LayUI 2.x + ECharts），含 axum handler、静态资源 rust-embed 嵌入、AJAX 数据源、SSE 实时订阅、导入按钮等前端交互。

**Architecture:** 视图层位于 `crates/devnpc-dashboard/src/views/` 目录，使用 askama 编译期模板（`layout.html` 公共布局 + 7 个页面模板继承）。axum handler 调用 askama 渲染 HTML 骨架，前端 JS 通过 AJAX 拉取阶段 3 已有的 `/api/*` 端点动态填充数据。LayUI 静态文件由 rust-embed 编译期嵌入 `static/` 目录，运行时零文件依赖。

**Tech Stack:** Rust 2024 edition, askama 0.12（编译期模板）, rust-embed 8（静态资源嵌入）, axum 0.7, LayUI 2.x（用户后续放入 static/）, ECharts, EventSource API

**关联 spec:** [2026-08-03-devnpc-dashboard-design.md](../specs/2026-08-03-devnpc-dashboard-design.md) §5.6 视图层 / §六 前端实现

**前置条件（阶段 3 已完成）:**
- `crates/devnpc-dashboard` crate 已在根 `Cargo.toml` 的 `[workspace] members` 中
- 已有 `src/server/{mod.rs, routes.rs, api.rs}`、`src/storage/{mod.rs, schema.rs, queries.rs}`、`src/realtime/mod.rs`、`src/auth.rs`、`src/error.rs`、`src/main.rs`
- 已定义 `AppState`（含 `storage: Arc<Storage>` 等字段）、`Storage` 的查询方法、`DashboardError`
- 已有 `/api/tasks`、`/api/tasks/:id`、`/api/tasks/:id/events`、`/api/stats/*`、`/api/realtime/stream`、`/api/events/import` 等 API 端点

**字段假设（阶段 3 已定义）:**
- `TaskRow`：`task_id`、`project`、`mr_iid: Option<i64>`、`pipeline_id: Option<i64>`、`task_description`、`task_kind`、`model`、`status`、`started_at`、`finished_at: Option<String>`、`duration_secs: Option<i64>`、`total_tokens: i64`、`input_tokens: i64`、`output_tokens: i64`、`estimated_cost_usd: f64`、`mr_url: Option<String>`、`ci_url: Option<String>`、`summary: Option<String>`、`error: Option<String>`、`ci_retries: i64`

---

## 文件结构总览

本阶段完成后目录结构：

```
crates/devnpc-dashboard/
├── Cargo.toml                         # 新增 askama/rust-embed/mime_guess 依赖
├── src/
│   ├── main.rs                        # (阶段 3 已创建, 不修改)
│   ├── server/
│   │   ├── mod.rs                     # (阶段 3 已创建, 不修改)
│   │   ├── routes.rs                  # 修改: 新增 7 个页面 handler + 静态资源 handler
│   │   ├── api.rs                     # (阶段 3 已创建, 不修改)
│   │   └── views.rs                   # 新增: askama 模板 struct 定义
│   ├── views/                         # 新增: askama HTML 模板目录
│   │   ├── layout.html                # 公共布局 (侧边栏 + 顶部 + LayUI 引入)
│   │   ├── index.html                 # 任务列表
│   │   ├── task_detail.html           # 任务详情 (时间线)
│   │   ├── realtime.html              # 实时监控 (SSE)
│   │   ├── trends.html                # 趋势统计 (4 个 ECharts)
│   │   ├── cost.html                  # 成本分析 (饼图 + 表格)
│   │   ├── ci.html                    # CI 自愈统计
│   │   └── sop.html                   # SOP 偏离监控
│   ├── static_files.rs                # 新增: rust-embed 嵌入 static/ 目录
│   ├── error.rs                       # (阶段 3 已创建, 不修改)
│   ├── auth.rs                        # (阶段 3 已创建, 不修改)
│   ├── storage/                       # (阶段 3 已创建, 不修改)
│   └── realtime/                      # (阶段 3 已创建, 不修改)
├── static/                            # 新增: 静态资源目录 (用户后续放入 LayUI)
│   ├── .gitkeep                       # 占位文件, 确保 git 跟踪空目录
│   ├── layui/                         # 用户后续放入 LayUI 框架
│   ├── css/
│   └── js/
└── tests/
    └── view_handlers.rs               # 新增: 页面 handler 集成测试
```

**设计要点:**
- askama 模板 path 通过 `[package.metadata.askama] template_dirs = ["src/views"]` 配置，使 `#[template(path = "index.html")]` 查找 `src/views/index.html`
- rust-embed 嵌入整个 `static/` 目录，运行时单二进制部署零文件依赖
- 前端 JS 全部内嵌在 HTML 模板的 `<script>` 标签中，无需单独 JS 文件
- LayUI / ECharts 静态文件由用户后续放入 `static/`，本阶段只创建目录占位

---

### Task 1: 添加 askama / rust-embed 依赖并创建 static 目录占位

**Files:**
- Modify: `crates/devnpc-dashboard/Cargo.toml`
- Create: `crates/devnpc-dashboard/static/.gitkeep`

- [ ] **Step 1: 修改 devnpc-dashboard/Cargo.toml 添加依赖和 askama 配置**

打开 `crates/devnpc-dashboard/Cargo.toml`，在 `[dependencies]` 段末尾追加三个依赖：

```toml
askama = "0.12"
rust-embed = "8"
mime_guess = "2"
```

在文件末尾追加 askama 模板目录配置（注意：必须放在 `[package.metadata]` 段下，让 askama 编译期查找 `src/views/` 目录）：

```toml
[package.metadata.askama]
template_dirs = ["src/views"]
```

完整修改后的 `[dependencies]` 段示例（在阶段 3 已有依赖基础上新增最后三行）：

```toml
[dependencies]
devnpc-core = { path = "../devnpc-core" }
axum = { version = "0.7", features = ["multipart"] }
tokio = { version = "1", features = ["full"] }
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
thiserror = "2"
clap = { version = "4", features = ["derive", "env"] }
tower = "0.5"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
dotenvy = "0.15"
# Phase 4 新增: 视图层
askama = "0.12"
rust-embed = "8"
mime_guess = "2"
```

- [ ] **Step 2: 创建 static 目录及占位文件**

Run: `mkdir crates\devnpc-dashboard\static ; mkdir crates\devnpc-dashboard\static\css ; mkdir crates\devnpc-dashboard\static\js`

创建 `crates/devnpc-dashboard/static/.gitkeep` 文件，内容为占位说明：

```
此目录由 rust-embed 在编译期嵌入到 devnpc-dashboard 二进制中。
用户需自行下载 LayUI 2.x 并放入 static/layui/ 子目录，下载 ECharts 放入 static/js/echarts.min.js。
预期目录结构:
  static/
    layui/        <- LayUI 2.x 解压后的目录 (含 layui.js / css/layui.css 等)
    css/
      dashboard.css
    js/
      echarts.min.js
```

创建 `crates/devnpc-dashboard/static/css/dashboard.css` 文件（提供基本样式，即使 LayUI 未放入也能展示页面）：

```css
/* devnpc Dashboard 自定义样式 */
.layui-logo {
    font-size: 20px;
    color: #fff;
    line-height: 60px;
    padding-left: 20px;
}
.layui-body {
    padding: 15px;
}
.log-area {
    max-height: 300px;
    overflow-y: auto;
    background: #fafafa;
    padding: 8px;
    font-family: Consolas, Monaco, monospace;
    font-size: 12px;
    border: 1px solid #eee;
}
.log-line {
    padding: 2px 0;
    border-bottom: 1px dashed #eee;
}
.log-time {
    color: #999;
    margin-right: 8px;
}
.log-type {
    color: #1E9FFF;
    margin-right: 8px;
    font-weight: bold;
}
.stat-card-value {
    font-size: 28px;
    font-weight: bold;
}
```

- [ ] **Step 3: 验证依赖解析**

Run: `cargo check -p devnpc-dashboard`
Expected: 编译通过（如果阶段 3 代码无误）。如有 `askama` 相关报错，确认 `template_dirs` 配置已正确添加但暂未使用（此 Task 还未创建模板）。

- [ ] **Step 4: 提交依赖和静态目录占位**

Run: `git add crates/devnpc-dashboard/Cargo.toml crates/devnpc-dashboard/Cargo.lock crates/devnpc-dashboard/static ; git commit -m "feat(dashboard): 添加 askama/rust-embed/mime_guess 依赖,创建 static 目录占位"`

---

### Task 2: 创建 rust-embed 静态资源嵌入模块

**Files:**
- Create: `crates/devnpc-dashboard/src/static_files.rs`
- Modify: `crates/devnpc-dashboard/src/main.rs`（或 `lib.rs`， whichever 中注册了 `mod server;`）

- [ ] **Step 1: 创建 src/static_files.rs**

```rust
//! 静态资源嵌入模块
//!
//! 使用 rust-embed 在编译期将 static/ 目录嵌入到二进制中,
//! 运行时零文件依赖。LayUI/ECharts 等静态文件由用户放入 static/ 后,
//! 下次 cargo build 自动嵌入。

use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

/// 嵌入 static/ 目录的所有文件
#[derive(RustEmbed)]
#[folder = "static/"]
struct StaticAsset;

/// 处理 GET /static/*path 请求,返回嵌入的静态文件
///
/// path 示例: "layui/layui.js" / "css/dashboard.css" / "js/echarts.min.js"
pub fn serve_static(path: &str) -> Response {
    match StaticAsset::get(path) {
        Some(file) => {
            // 通过文件扩展名推断 Content-Type
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let body = Body::from(file.data.into_owned());
            let mut resp = (StatusCode::OK, body).into_response();
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime.essence_str()).unwrap_or(HeaderValue::from_static("application/octet-stream")),
            );
            // 静态资源缓存 1 小时
            resp.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=3600"),
            );
            resp
        }
        None => (StatusCode::NOT_FOUND, "静态资源不存在").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_known_asset_returns_ok() {
        // dashboard.css 由 Task 1 Step 2 创建
        let resp = serve_static("css/dashboard.css");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn serve_unknown_asset_returns_404() {
        let resp = serve_static("not-exist-file.xyz");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn css_asset_has_text_css_mime() {
        let resp = serve_static("css/dashboard.css");
        let ct = resp.headers().get(header::CONTENT_TYPE).unwrap();
        assert_eq!(ct, "text/css");
    }
}
```

- [ ] **Step 2: 在 main.rs 中注册 static_files 模块**

打开 `crates/devnpc-dashboard/src/main.rs`，在已有的 `mod server;`、`mod storage;`、`mod realtime;`、`mod auth;`、`mod error;` 等声明附近，添加一行：

```rust
mod static_files;
```

- [ ] **Step 3: 验证编译和测试**

Run: `cargo test -p devnpc-dashboard --lib static_files`
Expected: 3 个测试全部 PASS

- [ ] **Step 4: 提交 rust-embed 模块**

Run: `git add crates/devnpc-dashboard/src/static_files.rs crates/devnpc-dashboard/src/main.rs ; git commit -m "feat(dashboard): 添加 rust-embed 静态资源嵌入模块 (serve_static handler)"`

---

### Task 3: 创建 layout.html 公共布局 + 模板 struct 骨架

**Files:**
- Create: `crates/devnpc-dashboard/src/views/layout.html`
- Create: `crates/devnpc-dashboard/src/server/views.rs`
- Modify: `crates/devnpc-dashboard/src/server/mod.rs`

- [ ] **Step 1: 创建 src/views/layout.html 公共布局**

```html
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{% block title %}devnpc Dashboard{% endblock %}</title>
    <!-- LayUI CSS (用户后续放入 static/layui/) -->
    <link rel="stylesheet" href="/static/layui/css/layui.css">
    <!-- 自定义样式 -->
    <link rel="stylesheet" href="/static/css/dashboard.css">
</head>
<body class="layui-layout-body">
<div class="layui-layout layui-layout-admin">
    <!-- 顶部导航 -->
    <div class="layui-header">
        <div class="layui-logo">devnpc Dashboard</div>
        <ul class="layui-nav layui-layout-left">
            <li class="layui-nav-item"><a href="/">任务列表</a></li>
            <li class="layui-nav-item"><a href="/realtime">实时监控</a></li>
        </ul>
    </div>

    <!-- 侧边栏导航 -->
    <div class="layui-side layui-bg-black">
        <div class="layui-side-scroll">
            <ul class="layui-nav layui-nav-tree" lay-filter="side-nav">
                <li class="layui-nav-item {% if active_nav == "tasks" %}layui-this{% endif %}">
                    <a href="/">任务列表</a>
                </li>
                <li class="layui-nav-item {% if active_nav == "realtime" %}layui-this{% endif %}">
                    <a href="/realtime">实时监控</a>
                </li>
                <li class="layui-nav-item {% if active_nav == "trends" %}layui-this{% endif %}">
                    <a href="/trends">趋势统计</a>
                </li>
                <li class="layui-nav-item {% if active_nav == "cost" %}layui-this{% endif %}">
                    <a href="/cost">成本分析</a>
                </li>
                <li class="layui-nav-item {% if active_nav == "ci" %}layui-this{% endif %}">
                    <a href="/ci">CI 自愈</a>
                </li>
                <li class="layui-nav-item {% if active_nav == "sop" %}layui-this{% endif %}">
                    <a href="/sop">SOP 偏离</a>
                </li>
            </ul>
        </div>
    </div>

    <!-- 主内容区 -->
    <div class="layui-body">
        <div class="layui-card">
            <div class="layui-card-header">
                <h2>{% block page_title %}{% endblock %}</h2>
            </div>
            <div class="layui-card-body">
                {% block content %}{% endblock %}
            </div>
        </div>
    </div>
</div>

<!-- LayUI JS (用户后续放入 static/layui/) -->
<script src="/static/layui/layui.js"></script>
<script>
// 全局 LayUI 模块加载
layui.use(['element', 'layer'], function(){
    var element = layui.element;
});
</script>

<!-- 子页面专属脚本 -->
{% block scripts %}{% endblock %}
</body>
</html>
```

- [ ] **Step 2: 创建 src/server/views.rs 模板 struct 定义**

```rust
//! askama 模板对应的 Rust struct 定义
//!
//! 每个页面一个 struct,通过 #[derive(Template)] 关联 HTML 模板。
//! 模板文件位于 src/views/,通过 Cargo.toml 中 [package.metadata.askama] 配置。

use askama::Template;

use crate::storage::queries::TaskRow;

/// 公共布局字段 (所有页面模板共享)
/// 通过 askama 的 extends 继承 layout.html,子模板自动获得这些字段。
#[derive(Template)]
#[template(path = "layout.html")]
pub struct LayoutTemplate {
    /// 当前激活的导航项标识: tasks/realtime/trends/cost/ci/sop
    pub active_nav: String,
}

/// 任务列表页
#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
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

/// CI 自愈统计页
#[derive(Template)]
#[template(path = "ci.html")]
pub struct CiTemplate {
    pub active_nav: String,
}

/// SOP 偏离监控页
#[derive(Template)]
#[template(path = "sop.html")]
pub struct SopTemplate {
    pub active_nav: String,
}
```

- [ ] **Step 3: 在 server/mod.rs 中注册 views 模块**

打开 `crates/devnpc-dashboard/src/server/mod.rs`，在已有的 `pub mod routes;`、`pub mod api;` 等声明附近，添加：

```rust
pub mod views;
```

- [ ] **Step 4: 验证编译（此时期望报错：模板文件不存在）**

Run: `cargo check -p devnpc-dashboard`
Expected: 报错 `Template 'index.html' not found` 或类似（因为 Task 3 只创建了 layout.html，其他模板还未创建）。这是正常的，后续 Task 会逐个创建。

如果报错是 askama 配置相关（如 `template_dirs` 无法解析），请确认 `Cargo.toml` 中 `[package.metadata.askama] template_dirs = ["src/views"]` 已正确添加。

- [ ] **Step 5: 临时注释未创建的模板 struct 验证 layout 编译**

为了让 Task 3 单独可编译通过（在 Task 4-10 创建各页面模板之前），暂时在 `views.rs` 中**仅保留** `LayoutTemplate` 和 `IndexTemplate`/`RealtimeTemplate` 等暂时不创建模板的 struct，将其他 struct 注释掉。

但更简单的做法是：**直接进入 Task 4 创建第一个子页面模板**。本 Task 不单独 commit，与 Task 4 一起提交。

- [ ] **Step 6: 不单独提交，进入 Task 4**

本 Task 创建的 `layout.html` 和 `views.rs` 骨架将与 Task 4 的 `index.html` 一起在 Task 4 末尾提交。

---

### Task 4: 任务列表页（index.html + handler + 路由）

**Files:**
- Create: `crates/devnpc-dashboard/src/views/index.html`
- Modify: `crates/devnpc-dashboard/src/server/routes.rs`

**spec 引用:** §6.3.1 任务列表 `/`

- [ ] **Step 1: 创建 src/views/index.html**

```html
{% extends "layout.html" %}

{% block title %}任务列表 - devnpc Dashboard{% endblock %}
{% block page_title %}任务列表（最近 100 条）{% endblock %}

{% block content %}
<!-- 工具栏:导入按钮 + 手动刷新 -->
<div class="layui-btn-container" style="margin-bottom: 10px;">
    <button class="layui-btn" id="btn-import">
        <i class="layui-icon layui-icon-upload"></i> 导入事件文件
    </button>
    <button class="layui-btn layui-btn-primary" id="btn-refresh">
        <i class="layui-icon layui-icon-refresh"></i> 刷新
    </button>
    <span class="layui-badge-rim" id="running-tip" style="display:none;margin-left:10px;">
        有运行中任务,5 秒后自动刷新
    </span>
</div>

<!-- 任务表格:AJAX 数据源 /api/tasks -->
<table id="tasks-table" lay-filter="tasks"></table>
{% endblock %}

{% block scripts %}
<script>
layui.use(['table', 'upload', 'layer'], function(){
    var table = layui.table;
    var upload = layui.upload;
    var layer = layui.layer;

    // 渲染任务表格
    table.render({
        elem: '#tasks-table',
        url: '/api/tasks',
        method: 'get',
        page: true,
        limit: 20,
        limits: [20, 50, 100],
        cols: [[
            {field: 'status', title: '状态', width: 110, templet: function(d){
                // 状态 badge:成功=绿/失败=红/运行=蓝/超时=橙/CI 失败=橙
                var map = {
                    'success':    '<span class="layui-badge layui-bg-green">成功</span>',
                    'failed':     '<span class="layui-badge">失败</span>',
                    'running':    '<span class="layui-badge layui-bg-blue">运行中</span>',
                    'timeout':    '<span class="layui-badge layui-bg-orange">超时</span>',
                    'ci_failed':  '<span class="layui-badge layui-bg-orange">CI 失败</span>'
                };
                return map[d.status] || d.status;
            }},
            {field: 'project', title: '项目', width: 200},
            {field: 'task_description', title: '任务描述'},
            {field: 'duration_secs', title: '耗时(秒)', width: 110, sort: true},
            {field: 'total_tokens', title: 'Token', width: 120, sort: true},
            {field: 'estimated_cost_usd', title: '成本($)', width: 110, sort: true, templet: function(d){
                return Number(d.estimated_cost_usd || 0).toFixed(4);
            }},
            {field: 'started_at', title: '开始时间', width: 180}
        ]],
        // 渲染完成后检查是否有 running 任务
        done: function(res) {
            var hasRunning = (res.data || []).some(function(t){
                return t.status === 'running';
            });
            var tip = document.getElementById('running-tip');
            if (hasRunning) {
                tip.style.display = 'inline-block';
                // 5 秒自动刷新 (仅当有 running 任务)
                setTimeout(function(){
                    table.reload('tasks-table');
                }, 5000);
            } else {
                tip.style.display = 'none';
            }
        }
    });

    // 点击行跳转任务详情
    table.on('row(tasks)', function(obj){
        window.location.href = '/tasks/' + obj.data.task_id;
    });

    // 导入事件文件:layui-upload 文件上传
    upload.render({
        elem: '#btn-import',
        url: '/api/events/import',
        accept: 'file',
        exts: 'jsonl',
        headers: {
            // 导入接口需要 token,从 cookie 或 URL 查询参数获取
            // 这里假设 dashboard 查看侧无需 token (spec §5.2 鉴权表: 导入接口需 token)
            // 实际部署时通过 URL 参数 ?token=xxx 传递,服务端从查询参数读取
        },
        done: function(res){
            // res 是服务端返回的 JSON
            if (res.imported) {
                layer.msg('导入成功,共 ' + res.events_count + ' 条事件', {icon: 1});
                table.reload('tasks-table');
            } else if (res.skipped) {
                layer.msg('任务已存在,跳过导入', {icon: 0});
            } else {
                layer.msg('导入失败: ' + (res.error || '未知错误'), {icon: 2});
            }
        },
        error: function(){
            layer.msg('上传失败,请检查网络或文件格式(.jsonl)', {icon: 2});
        }
    });

    // 手动刷新按钮
    document.getElementById('btn-refresh').addEventListener('click', function(){
        table.reload('tasks-table');
    });
});
</script>
{% endblock %}
```

- [ ] **Step 2: 在 src/server/routes.rs 添加 index_page handler**

打开 `crates/devnpc-dashboard/src/server/routes.rs`，在文件顶部确认有以下 imports（如缺则补齐）：

```rust
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use askama::Template;

use crate::error::DashboardError;
use crate::server::views::{IndexTemplate, TaskDetailTemplate, RealtimeTemplate, TrendsTemplate, CostTemplate, CiTemplate, SopTemplate};
use crate::static_files::serve_static;
use crate::storage::queries::TaskRow;
```

注意：此步骤中导入的所有 Template struct 还未全部定义（Task 5-10 才创建），所以本 Task 暂时只 import `IndexTemplate`。后续 Task 完成时再补全 import 列表。

为了避免编译报错，本 Task 暂时只导入 `IndexTemplate`：

```rust
use crate::server::views::IndexTemplate;
```

在 routes.rs 末尾追加 handler：

```rust
/// GET / - 任务列表页
pub async fn index_page() -> Result<Html<String>, DashboardError> {
    let tmpl = IndexTemplate {
        active_nav: "tasks".to_string(),
    };
    let html = tmpl.render()?;
    Ok(Html(html))
}
```

- [ ] **Step 3: 在 router 中注册 GET / 路由**

在 routes.rs 中查找阶段 3 已有的 `pub fn router(state: AppState) -> axum::Router` 函数（或类似命名的 router 构建函数），在页面路由部分添加：

```rust
.route("/", get(index_page))
```

如果阶段 3 的 router 使用 `axum::Router::new()` 链式调用，找到对应位置插入上述 `.route()`。完整示例（假设阶段 3 已有 router 函数）：

```rust
pub fn router(state: AppState) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        // === 页面路由 (Phase 4 新增) ===
        .route("/", get(index_page))
        // === API 路由 (阶段 3 已有) ===
        .route("/api/tasks", get(crate::server::api::list_tasks))
        .route("/api/tasks/:id", get(crate::server::api::get_task))
        .route("/api/tasks/:id/events", get(crate::server::api::list_events))
        // ... 其他已有路由
        .with_state(state)
}
```

- [ ] **Step 4: 验证编译和模板渲染**

Run: `cargo check -p devnpc-dashboard`
Expected: 编译通过。如报错 `cannot find type IndexTemplate`，确认 `views.rs` 中已定义且 `server/mod.rs` 中已 `pub mod views;`。

Run: `cargo build -p devnpc-dashboard`
Expected: 构建成功，二进制生成在 `target/debug/devnpc-dashboard.exe`。

- [ ] **Step 5: 手动验证页面可访问**

Run: `.\target\debug\devnpc-dashboard.exe --port 18080 --db .\test-dashboard.db --token test-token-123`

新开终端:
Run: `Invoke-WebRequest -Uri http://localhost:18080/ -UseBasicParsing | Select-Object -ExpandProperty Content`
Expected: 返回包含 `<title>任务列表 - devnpc Dashboard</title>` 和 `layui-layout-admin` 类的 HTML。停止服务: `Ctrl+C`。

- [ ] **Step 6: 提交任务列表页**

Run: `git add crates/devnpc-dashboard/src/views/layout.html crates/devnpc-dashboard/src/views/index.html crates/devnpc-dashboard/src/server/views.rs crates/devnpc-dashboard/src/server/mod.rs crates/devnpc-dashboard/src/server/routes.rs ; git commit -m "feat(dashboard): 实现任务列表页 (index.html + index_page handler + GET / 路由)"`

---

### Task 5: 任务详情页（task_detail.html + handler + 路由）

**Files:**
- Create: `crates/devnpc-dashboard/src/views/task_detail.html`
- Modify: `crates/devnpc-dashboard/src/server/views.rs`（已在 Task 3 定义 `TaskDetailTemplate`，确认即可）
- Modify: `crates/devnpc-dashboard/src/server/routes.rs`

**spec 引用:** §6.3.2 任务详情 `/tasks/:id`

- [ ] **Step 1: 创建 src/views/task_detail.html**

```html
{% extends "layout.html" %}

{% block title %}任务详情 - {{ task.task_description }}{% endblock %}
{% block page_title %}任务详情{% endblock %}

{% block content %}
<div class="layui-row layui-col-space15">
    <!-- 任务元信息卡片 -->
    <div class="layui-col-md12">
        <div class="layui-card">
            <div class="layui-card-header">任务元信息</div>
            <div class="layui-card-body">
                <table class="layui-table">
                    <colgroup><col width="160"><col></colgroup>
                    <tbody>
                        <tr><td>任务 ID</td><td><code>{{ task.task_id }}</code></td></tr>
                        <tr><td>状态</td><td>{{ task.status }}</td></tr>
                        <tr><td>项目</td><td>{{ task.project }}</td></tr>
                        <tr><td>任务描述</td><td>{{ task.task_description }}</td></tr>
                        <tr><td>类型</td><td>{{ task.task_kind }}</td></tr>
                        <tr><td>模型</td><td>{{ task.model }}</td></tr>
                        <tr><td>耗时</td><td>{{ task.duration_secs.unwrap_or(0) }} 秒</td></tr>
                        <tr><td>Token</td><td>{{ task.total_tokens }} (输入 {{ task.input_tokens }} / 输出 {{ task.output_tokens }})</td></tr>
                        <tr><td>成本</td><td>${{ task.estimated_cost_usd }}</td></tr>
                        <tr><td>开始时间</td><td>{{ task.started_at }}</td></tr>
                        <tr><td>结束时间</td><td>
                            {% if let Some(f) = task.finished_at.as_deref() %}
                                {{ f }}
                            {% else %}
                                <span class="layui-badge layui-bg-blue">运行中</span>
                            {% endif %}
                        </td></tr>
                        {% if let Some(url) = task.mr_url.as_deref() %}
                        <tr><td>MR 链接</td><td><a href="{{ url }}" target="_blank">{{ url }}</a></td></tr>
                        {% endif %}
                        {% if let Some(url) = task.ci_url.as_deref() %}
                        <tr><td>CI 链接</td><td><a href="{{ url }}" target="_blank">{{ url }}</a></td></tr>
                        {% endif %}
                        {% if let Some(s) = task.summary.as_deref() %}
                        <tr><td>验收摘要</td><td>{{ s }}</td></tr>
                        {% endif %}
                        {% if let Some(e) = task.error.as_deref() %}
                        <tr><td>错误信息</td><td><span style="color:red;">{{ e }}</span></td></tr>
                        {% endif %}
                    </tbody>
                </table>
            </div>
        </div>
    </div>

    <!-- 执行时间线 -->
    <div class="layui-col-md12">
        <div class="layui-card">
            <div class="layui-card-header">执行时间线</div>
            <div class="layui-card-body">
                <ul class="layui-timeline" id="timeline">
                    <li class="layui-timeline-item">
                        <i class="layui-icon layui-icon-loading-1"></i>
                        <div class="layui-timeline-content layui-text">加载中...</div>
                    </li>
                </ul>
            </div>
        </div>
    </div>
</div>
{% endblock %}

{% block scripts %}
<script>
layui.use(['layer'], function(){
    var layer = layui.layer;

    // 从 URL 提取 task_id (路径形如 /tasks/{id})
    var taskId = window.location.pathname.split('/').pop();

    // 加载事件列表
    fetch('/api/tasks/' + taskId + '/events')
        .then(function(r){
            if (!r.ok) throw new Error('HTTP ' + r.status);
            return r.json();
        })
        .then(function(events){
            if (!events || events.length === 0) {
                document.getElementById('timeline').innerHTML =
                    '<li class="layui-timeline-item"><div class="layui-timeline-content">暂无事件</div></li>';
                return;
            }
            // 不同事件类型用不同图标和颜色 (spec §6.3.2)
            var iconMap = {
                'llm_call':      {icon: 'layui-icon-chat',       color: '#1E9FFF'},
                'tool_call':     {icon: 'layui-icon-util',       color: '#5FB878'},
                'sop_step':      {icon: 'layui-icon-template-1', color: '#FFB800'},
                'ci_status':     {icon: 'layui-icon-templei-1',  color: '#01AAED'},
                'team_handoff':  {icon: 'layui-icon-username',   color: '#FF5722'}
            };

            var html = events.map(function(e){
                var cfg = iconMap[e.event_type] || {icon: 'layui-icon-circle-dot', color: '#999'};
                var payload = {};
                try { payload = JSON.parse(e.payload); } catch(_) {}
                var detail = '';
                // 根据事件类型构造详情文本
                switch (e.event_type) {
                    case 'llm_call':
                        detail = '第 ' + payload.iteration + ' 次调用, prompt=' +
                                 payload.prompt_tokens + ', completion=' +
                                 payload.completion_tokens + ', 延时 ' + payload.latency_ms + 'ms';
                        break;
                    case 'tool_call':
                        detail = '工具 <code>' + payload.name + '</code> ' +
                                 (payload.success ? '成功' : '失败') +
                                 ' (' + payload.latency_ms + 'ms) ' + payload.detail;
                        break;
                    case 'sop_step':
                        detail = '步骤 <code>' + payload.step + '</code> 状态: ' +
                                 payload.status + (payload.note ? ' - ' + payload.note : '');
                        break;
                    case 'ci_status':
                        detail = 'Pipeline #' + payload.pipeline_id + ' ' + payload.status +
                                 ' (第 ' + payload.attempt + ' 次重试)';
                        break;
                    case 'team_handoff':
                        detail = payload.from_role + ' → ' + payload.to_role +
                                 ' (signal: ' + payload.signal + ')';
                        break;
                    default:
                        detail = e.payload;
                }
                return '<li class="layui-timeline-item">' +
                    '<i class="layui-icon ' + cfg.icon + '" style="color:' + cfg.color + ';font-size:20px;"></i>' +
                    '<div class="layui-timeline-content layui-text">' +
                    '<h3 class="layui-timeline-title">' + e.created_at + '</h3>' +
                    '<p><strong>' + e.event_type + '</strong> - ' + detail + '</p>' +
                    '</div></li>';
            }).join('');
            document.getElementById('timeline').innerHTML = html;
        })
        .catch(function(err){
            layer.msg('加载事件失败: ' + err.message, {icon: 2});
            document.getElementById('timeline').innerHTML =
                '<li class="layui-timeline-item"><div class="layui-timeline-content" style="color:red;">加载失败: ' + err.message + '</div></li>';
        });
});
</script>
{% endblock %}
```

- [ ] **Step 2: 确认 views.rs 中 TaskDetailTemplate 已定义**

打开 `crates/devnpc-dashboard/src/server/views.rs`，确认 Task 3 已定义：

```rust
#[derive(Template)]
#[template(path = "task_detail.html")]
pub struct TaskDetailTemplate {
    pub active_nav: String,
    pub task: TaskRow,
}
```

如未定义则补上。注意 `TaskRow` 必须实现 `askama::Template` 不需要——它只需要在模板中可访问字段。askama 会通过字段访问语法 `{{ task.task_id }}` 渲染，要求 `TaskRow` 的字段是 pub 且实现 `Display`（String/i64/f64/Option 都满足）。

- [ ] **Step 3: 在 routes.rs 添加 task_detail_page handler**

在 `routes.rs` 追加（紧接 `index_page` 之后）：

```rust
/// GET /tasks/:id - 任务详情页
pub async fn task_detail_page(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Html<String>, DashboardError> {
    let task = state
        .storage
        .get_task(&task_id)?
        .ok_or_else(|| DashboardError::TaskNotFound(task_id.clone()))?;
    let tmpl = TaskDetailTemplate {
        active_nav: "tasks".to_string(),
        task,
    };
    let html = tmpl.render()?;
    Ok(Html(html))
}
```

注意：此 handler 依赖阶段 3 已定义的 `Storage::get_task`（返回 `Result<Option<TaskRow>>`）和 `DashboardError::TaskNotFound`。

- [ ] **Step 4: 更新 routes.rs 顶部 import，添加 TaskDetailTemplate**

修改 `routes.rs` 顶部 import（将 Task 4 Step 2 中的 `use crate::server::views::IndexTemplate;` 扩展为）：

```rust
use crate::server::views::{IndexTemplate, TaskDetailTemplate};
```

- [ ] **Step 5: 在 router 中注册 GET /tasks/:id 路由**

在 `routes.rs` 的 `router()` 函数中，紧接 `.route("/", get(index_page))` 之后添加：

```rust
.route("/tasks/:id", get(task_detail_page))
```

- [ ] **Step 6: 验证编译**

Run: `cargo check -p devnpc-dashboard`
Expected: 编译通过。如报错 `cannot find type TaskRow`，在 `views.rs` 顶部确认 `use crate::storage::queries::TaskRow;`。

Run: `cargo build -p devnpc-dashboard`
Expected: 构建成功。

- [ ] **Step 7: 提交任务详情页**

Run: `git add crates/devnpc-dashboard/src/views/task_detail.html crates/devnpc-dashboard/src/server/views.rs crates/devnpc-dashboard/src/server/routes.rs ; git commit -m "feat(dashboard): 实现任务详情页 (task_detail.html + handler + 时间线渲染)"`

---

### Task 6: 实时监控页（realtime.html + handler + 路由）

**Files:**
- Create: `crates/devnpc-dashboard/src/views/realtime.html`
- Modify: `crates/devnpc-dashboard/src/server/routes.rs`

**spec 引用:** §6.3.3 实时监控 `/realtime`

- [ ] **Step 1: 创建 src/views/realtime.html**

```html
{% extends "layout.html" %}

{% block title %}实时监控 - devnpc Dashboard{% endblock %}
{% block page_title %}实时监控（SSE 推送）{% endblock %}

{% block content %}
<div class="layui-btn-container" style="margin-bottom:10px;">
    <button class="layui-btn layui-btn-sm" id="btn-clear">清空日志</button>
    <span class="layui-badge-rim" id="sse-status" style="margin-left:10px;">连接中...</span>
</div>

<!-- 折叠面板容器:每个 running 任务一个面板 -->
<div class="layui-collapse" id="realtime-panel" lay-filter="realtime"></div>

<!-- 空状态提示 -->
<div id="empty-tip" style="padding:60px;text-align:center;color:#999;">
    <i class="layui-icon layui-icon-loading-1" style="font-size:30px;"></i>
    <p>等待运行中的任务...</p>
</div>
{% endblock %}

{% block scripts %}
<script>
layui.use(['layer', 'element'], function(){
    var element = layui.element;
    var layer = layui.layer;

    var panels = {};  // task_id -> { lastUpdate, finished }

    // 订阅 SSE (spec §6.1: 原生 EventSource API)
    var source = new EventSource('/api/realtime/stream');

    source.onopen = function() {
        document.getElementById('sse-status').className = 'layui-badge layui-bg-green';
        document.getElementById('sse-status').textContent = '已连接';
    };

    source.onmessage = function(e) {
        var data = JSON.parse(e.data);
        var taskId = data.task_id;
        var eventType = data.event_type;
        var eventPayload = data.event || {};
        var timestamp = data.timestamp || new Date().toISOString();

        // 检测任务完成事件 (event_type=task_finished)
        var isFinished = (eventType === 'task_finished');

        // 首次见到该 task_id:创建折叠面板
        if (!panels[taskId]) {
            document.getElementById('empty-tip').style.display = 'none';
            var shortId = taskId.substring(0, 8);
            var html = '<div class="layui-colla-item" data-task="' + taskId + '" id="panel-' + shortId + '">' +
                '<h2 class="layui-colla-title">' +
                '<i class="layui-icon layui-icon-component"></i> 任务 ' + shortId +
                ' <span class="layui-badge layui-bg-blue" style="margin-left:8px;" id="badge-' + shortId + '">运行中</span>' +
                '</h2>' +
                '<div class="layui-colla-content layui-show">' +
                '<div class="log-area" id="log-' + shortId + '"></div>' +
                '</div></div>';
            var wrap = document.createElement('div');
            wrap.innerHTML = html;
            document.getElementById('realtime-panel').appendChild(wrap.firstChild);
            element.render('collapse');
            panels[taskId] = { shortId: shortId, finished: false };
        }

        var info = panels[taskId];
        var logArea = document.getElementById('log-' + info.shortId);

        // 追加日志行
        if (logArea) {
            var detail = '';
            if (typeof eventPayload === 'object') {
                // 简化显示:把对象 key/value 拼接
                detail = Object.keys(eventPayload).map(function(k){
                    return k + '=' + JSON.stringify(eventPayload[k]);
                }).join(' ');
            } else {
                detail = String(eventPayload);
            }
            var line = '<div class="log-line">' +
                '<span class="log-time">' + timestamp + '</span>' +
                '<span class="log-type">' + eventType + '</span>' +
                '<span class="log-detail">' + detail + '</span>' +
                '</div>';
            logArea.insertAdjacentHTML('beforeend', line);
            logArea.scrollTop = logArea.scrollHeight;
        }

        // 任务完成:面板边框变色,3 秒后自动收起 (spec §6.3.3)
        if (isFinished) {
            info.finished = true;
            var badge = document.getElementById('badge-' + info.shortId);
            if (badge) {
                badge.className = 'layui-badge layui-bg-gray';
                badge.textContent = '已完成';
            }
            var panel = document.getElementById('panel-' + info.shortId);
            if (panel) {
                panel.style.borderLeft = '3px solid #5FB878';
                setTimeout(function(){
                    // 通过模拟点击标题来收起
                    var title = panel.querySelector('.layui-colla-title');
                    if (title) title.click();
                }, 3000);
            }
        }
    };

    source.onerror = function() {
        document.getElementById('sse-status').className = 'layui-badge';
        document.getElementById('sse-status').textContent = '断开,重连中...';
        // EventSource 浏览器原生自动重连,无需手动处理 (spec §6.5)
    };

    // 清空日志按钮
    document.getElementById('btn-clear').addEventListener('click', function(){
        document.querySelectorAll('.log-area').forEach(function(el){
            el.innerHTML = '';
        });
    });
});
</script>
{% endblock %}
```

- [ ] **Step 2: 在 routes.rs 添加 realtime_page handler**

在 `routes.rs` 追加：

```rust
/// GET /realtime - 实时监控页
pub async fn realtime_page() -> Result<Html<String>, DashboardError> {
    let tmpl = RealtimeTemplate {
        active_nav: "realtime".to_string(),
    };
    let html = tmpl.render()?;
    Ok(Html(html))
}
```

- [ ] **Step 3: 更新 routes.rs 顶部 import，添加 RealtimeTemplate**

将 `use crate::server::views::{IndexTemplate, TaskDetailTemplate};` 扩展为：

```rust
use crate::server::views::{IndexTemplate, TaskDetailTemplate, RealtimeTemplate};
```

- [ ] **Step 4: 在 router 中注册 GET /realtime 路由**

在 `router()` 函数中添加：

```rust
.route("/realtime", get(realtime_page))
```

- [ ] **Step 5: 验证编译**

Run: `cargo check -p devnpc-dashboard`
Expected: 编译通过。

- [ ] **Step 6: 提交实时监控页**

Run: `git add crates/devnpc-dashboard/src/views/realtime.html crates/devnpc-dashboard/src/server/routes.rs ; git commit -m "feat(dashboard): 实现实时监控页 (realtime.html + SSE EventSource 订阅 + 折叠面板)"`

---

### Task 7: 趋势统计页（trends.html + handler + 路由）

**Files:**
- Create: `crates/devnpc-dashboard/src/views/trends.html`
- Modify: `crates/devnpc-dashboard/src/server/routes.rs`

**spec 引用:** §6.3.4 趋势统计 `/trends`

- [ ] **Step 1: 创建 src/views/trends.html**

```html
{% extends "layout.html" %}

{% block title %}趋势统计 - devnpc Dashboard{% endblock %}
{% block page_title %}趋势统计{% endblock %}

{% block content %}
<!-- 时间范围切换:7/30/90 天 -->
<div class="layui-btn-container" style="margin-bottom:15px;">
    <button class="layui-btn layui-btn-sm" data-days="7">最近 7 天</button>
    <button class="layui-btn layui-btn-sm layui-btn-primary" data-days="30">最近 30 天</button>
    <button class="layui-btn layui-btn-sm layui-btn-primary" data-days="90">最近 90 天</button>
</div>

<!-- 4 个 ECharts 图表:成功率/平均耗时/Token/成本 (spec §6.3.4) -->
<div class="layui-row layui-col-space15">
    <div class="layui-col-md6">
        <div class="layui-card">
            <div class="layui-card-header">任务成功率</div>
            <div class="layui-card-body">
                <div id="chart-success" style="height:300px;"></div>
            </div>
        </div>
    </div>
    <div class="layui-col-md6">
        <div class="layui-card">
            <div class="layui-card-header">平均耗时(秒)</div>
            <div class="layui-card-body">
                <div id="chart-duration" style="height:300px;"></div>
            </div>
        </div>
    </div>
    <div class="layui-col-md6">
        <div class="layui-card">
            <div class="layui-card-header">Token 消耗</div>
            <div class="layui-card-body">
                <div id="chart-tokens" style="height:300px;"></div>
            </div>
        </div>
    </div>
    <div class="layui-col-md6">
        <div class="layui-card">
            <div class="layui-card-header">成本($)</div>
            <div class="layui-card-body">
                <div id="chart-cost" style="height:300px;"></div>
            </div>
        </div>
    </div>
</div>
{% endblock %}

{% block scripts %}
<!-- ECharts (用户后续放入 static/js/echarts.min.js) -->
<script src="/static/js/echarts.min.js"></script>
<script>
var charts = {
    success: echarts.init(document.getElementById('chart-success')),
    duration: echarts.init(document.getElementById('chart-duration')),
    tokens: echarts.init(document.getElementById('chart-tokens')),
    cost: echarts.init(document.getElementById('chart-cost'))
};

// 加载趋势数据 (spec §5.2: GET /api/stats/trends?days=N)
function loadTrends(days) {
    fetch('/api/stats/trends?days=' + days)
        .then(function(r){
            if (!r.ok) throw new Error('HTTP ' + r.status);
            return r.json();
        })
        .then(function(data){
            var dates = data.dates || [];
            // 成功率折线图
            charts.success.setOption({
                tooltip: {trigger: 'axis'},
                xAxis: {type: 'category', data: dates},
                yAxis: {type: 'value', max: 100, axisLabel: {formatter: '{value}%'}},
                series: [{
                    name: '成功率',
                    type: 'line',
                    data: data.success_rates || [],
                    smooth: true,
                    itemStyle: {color: '#5FB878'}
                }]
            });
            // 平均耗时柱状图
            charts.duration.setOption({
                tooltip: {trigger: 'axis'},
                xAxis: {type: 'category', data: dates},
                yAxis: {type: 'value'},
                series: [{
                    name: '平均耗时',
                    type: 'bar',
                    data: data.avg_durations || [],
                    itemStyle: {color: '#1E9FFF'}
                }]
            });
            // Token 消耗面积图
            charts.tokens.setOption({
                tooltip: {trigger: 'axis'},
                xAxis: {type: 'category', data: dates},
                yAxis: {type: 'value'},
                series: [{
                    name: 'Token',
                    type: 'line',
                    data: data.total_tokens || [],
                    areaStyle: {},
                    itemStyle: {color: '#FFB800'}
                }]
            });
            // 成本面积图
            charts.cost.setOption({
                tooltip: {trigger: 'axis'},
                xAxis: {type: 'category', data: dates},
                yAxis: {type: 'value'},
                series: [{
                    name: '成本',
                    type: 'line',
                    data: data.total_costs || [],
                    areaStyle: {},
                    itemStyle: {color: '#FF5722'}
                }]
            });
        })
        .catch(function(err){
            layui.layer.msg('加载趋势数据失败: ' + err.message, {icon: 2});
        });
}

// 默认加载 7 天
loadTrends(7);

// 时间范围切换按钮
document.querySelectorAll('[data-days]').forEach(function(btn){
    btn.addEventListener('click', function(){
        // 切换按钮样式:当前按钮高亮,其他变 primary
        document.querySelectorAll('[data-days]').forEach(function(b){
            b.classList.add('layui-btn-primary');
        });
        this.classList.remove('layui-btn-primary');
        loadTrends(this.dataset.days);
    });
});

// 窗口尺寸变化时重绘图表
window.addEventListener('resize', function(){
    Object.keys(charts).forEach(function(k){ charts[k].resize(); });
});
</script>
{% endblock %}
```

- [ ] **Step 2: 在 routes.rs 添加 trends_page handler**

在 `routes.rs` 追加：

```rust
/// GET /trends - 趋势统计页
pub async fn trends_page() -> Result<Html<String>, DashboardError> {
    let tmpl = TrendsTemplate {
        active_nav: "trends".to_string(),
    };
    let html = tmpl.render()?;
    Ok(Html(html))
}
```

- [ ] **Step 3: 更新 routes.rs 顶部 import，添加 TrendsTemplate**

将 import 扩展为：

```rust
use crate::server::views::{IndexTemplate, TaskDetailTemplate, RealtimeTemplate, TrendsTemplate};
```

- [ ] **Step 4: 在 router 中注册 GET /trends 路由**

在 `router()` 函数中添加：

```rust
.route("/trends", get(trends_page))
```

- [ ] **Step 5: 验证编译**

Run: `cargo check -p devnpc-dashboard`
Expected: 编译通过。

- [ ] **Step 6: 提交趋势统计页**

Run: `git add crates/devnpc-dashboard/src/views/trends.html crates/devnpc-dashboard/src/server/routes.rs ; git commit -m "feat(dashboard): 实现趋势统计页 (trends.html + 4 个 ECharts 图表 + 7/30/90 天切换)"`

---

### Task 8: 成本分析页（cost.html + handler + 路由）

**Files:**
- Create: `crates/devnpc-dashboard/src/views/cost.html`
- Modify: `crates/devnpc-dashboard/src/server/routes.rs`

**spec 引用:** §6.3.5 成本分析 `/cost`

- [ ] **Step 1: 创建 src/views/cost.html**

```html
{% extends "layout.html" %}

{% block title %}成本分析 - devnpc Dashboard{% endblock %}
{% block page_title %}成本分析{% endblock %}

{% block content %}
<!-- 分组维度切换:项目/模型/任务类型 (spec §6.3.5) -->
<div class="layui-btn-container" style="margin-bottom:15px;">
    <button class="layui-btn layui-btn-sm" data-group="project">按项目</button>
    <button class="layui-btn layui-btn-sm layui-btn-primary" data-group="model">按模型</button>
    <button class="layui-btn layui-btn-sm layui-btn-primary" data-group="kind">按任务类型</button>
</div>

<div class="layui-row layui-col-space15">
    <!-- 饼图:成本占比 -->
    <div class="layui-col-md6">
        <div class="layui-card">
            <div class="layui-card-header">成本占比</div>
            <div class="layui-card-body">
                <div id="chart-pie" style="height:400px;"></div>
            </div>
        </div>
    </div>
    <!-- 明细表格 -->
    <div class="layui-col-md6">
        <div class="layui-card">
            <div class="layui-card-header">明细</div>
            <div class="layui-card-body">
                <table id="cost-table" lay-filter="cost"></table>
            </div>
        </div>
    </div>
</div>
{% endblock %}

{% block scripts %}
<script src="/static/js/echarts.min.js"></script>
<script>
layui.use(['table'], function(){
    var table = layui.table;
    var pieChart = echarts.init(document.getElementById('chart-pie'));

    // 加载成本数据 (spec §5.2: GET /api/stats/cost?group_by=xxx)
    function loadCost(groupBy) {
        fetch('/api/stats/cost?group_by=' + groupBy)
            .then(function(r){
                if (!r.ok) throw new Error('HTTP ' + r.status);
                return r.json();
            })
            .then(function(data){
                // data: 数组,每项 {label, task_count, total_tokens, cost}
                var pieData = (data || []).map(function(b){
                    return {name: b.label, value: b.cost};
                });
                // 饼图
                pieChart.setOption({
                    title: {text: '成本占比', left: 'center'},
                    tooltip: {trigger: 'item', formatter: '{b}: ${c} ({d}%)'},
                    series: [{
                        type: 'pie',
                        radius: '60%',
                        data: pieData,
                        emphasis: {itemStyle: {shadowBlur: 10, shadowOffsetX: 0, shadowColor: 'rgba(0, 0, 0, 0.5)'}}
                    }]
                });
                // 明细表格 (静态数据填充 layui-table)
                table.render({
                    elem: '#cost-table',
                    data: data || [],
                    cols: [[
                        {field: 'label', title: '名称'},
                        {field: 'task_count', title: '任务数', width: 100},
                        {field: 'total_tokens', title: 'Token', width: 120},
                        {field: 'cost', title: '成本($)', width: 120, templet: function(d){
                            return Number(d.cost || 0).toFixed(4);
                        }}
                    ]]
                });
            })
            .catch(function(err){
                layui.layer.msg('加载成本数据失败: ' + err.message, {icon: 2});
            });
    }

    // 默认按项目分组
    loadCost('project');

    // 分组维度切换
    document.querySelectorAll('[data-group]').forEach(function(btn){
        btn.addEventListener('click', function(){
            document.querySelectorAll('[data-group]').forEach(function(b){
                b.classList.add('layui-btn-primary');
            });
            this.classList.remove('layui-btn-primary');
            loadCost(this.dataset.group);
        });
    });

    // 窗口尺寸变化时重绘
    window.addEventListener('resize', function(){
        pieChart.resize();
    });
});
</script>
{% endblock %}
```

- [ ] **Step 2: 在 routes.rs 添加 cost_page handler**

在 `routes.rs` 追加：

```rust
/// GET /cost - 成本分析页
pub async fn cost_page() -> Result<Html<String>, DashboardError> {
    let tmpl = CostTemplate {
        active_nav: "cost".to_string(),
    };
    let html = tmpl.render()?;
    Ok(Html(html))
}
```

- [ ] **Step 3: 更新 routes.rs 顶部 import，添加 CostTemplate**

将 import 扩展为：

```rust
use crate::server::views::{IndexTemplate, TaskDetailTemplate, RealtimeTemplate, TrendsTemplate, CostTemplate};
```

- [ ] **Step 4: 在 router 中注册 GET /cost 路由**

在 `router()` 函数中添加：

```rust
.route("/cost", get(cost_page))
```

- [ ] **Step 5: 验证编译**

Run: `cargo check -p devnpc-dashboard`
Expected: 编译通过。

- [ ] **Step 6: 提交成本分析页**

Run: `git add crates/devnpc-dashboard/src/views/cost.html crates/devnpc-dashboard/src/server/routes.rs ; git commit -m "feat(dashboard): 实现成本分析页 (cost.html + ECharts 饼图 + 维度切换)"`

---

### Task 9: CI 自愈统计页（ci.html + handler + 路由）

**Files:**
- Create: `crates/devnpc-dashboard/src/views/ci.html`
- Modify: `crates/devnpc-dashboard/src/server/routes.rs`

**spec 引用:** §6.3.6 CI 自愈统计 `/ci`

- [ ] **Step 1: 创建 src/views/ci.html**

```html
{% extends "layout.html" %}

{% block title %}CI 自愈统计 - devnpc Dashboard{% endblock %}
{% block page_title %}CI 自愈统计{% endblock %}

{% block content %}
<!-- 概览卡片:总失败/自动修复/成功率/平均重试 (spec §6.3.6) -->
<div class="layui-row layui-col-space15">
    <div class="layui-col-md3">
        <div class="layui-card">
            <div class="layui-card-header">总失败任务</div>
            <div class="layui-card-body">
                <span class="stat-card-value" style="color:#FF5722;" id="stat-total-failed">-</span>
            </div>
        </div>
    </div>
    <div class="layui-col-md3">
        <div class="layui-card">
            <div class="layui-card-header">自动修复</div>
            <div class="layui-card-body">
                <span class="stat-card-value" style="color:#5FB878;" id="stat-auto-fixed">-</span>
            </div>
        </div>
    </div>
    <div class="layui-col-md3">
        <div class="layui-card">
            <div class="layui-card-header">修复成功率</div>
            <div class="layui-card-body">
                <span class="stat-card-value" style="color:#1E9FFF;" id="stat-success-rate">-</span>%
            </div>
        </div>
    </div>
    <div class="layui-col-md3">
        <div class="layui-card">
            <div class="layui-card-header">平均重试次数</div>
            <div class="layui-card-body">
                <span class="stat-card-value" style="color:#FFB800;" id="stat-avg-retries">-</span>
            </div>
        </div>
    </div>
</div>

<!-- ECharts 柱状图:重试次数分布 -->
<div class="layui-row layui-col-space15">
    <div class="layui-col-md12">
        <div class="layui-card">
            <div class="layui-card-header">重试次数分布</div>
            <div class="layui-card-body">
                <div id="chart-retry" style="height:300px;"></div>
            </div>
        </div>
    </div>
</div>

<!-- 失败任务列表 -->
<div class="layui-row layui-col-space15">
    <div class="layui-col-md12">
        <div class="layui-card">
            <div class="layui-card-header">失败任务列表</div>
            <div class="layui-card-body">
                <table id="failed-table" lay-filter="failed"></table>
            </div>
        </div>
    </div>
</div>
{% endblock %}

{% block scripts %}
<script src="/static/js/echarts.min.js"></script>
<script>
layui.use(['table'], function(){
    var table = layui.table;
    var chart = echarts.init(document.getElementById('chart-retry'));

    // 加载 CI 自愈统计 (spec §5.2: GET /api/stats/ci)
    fetch('/api/stats/ci')
        .then(function(r){
            if (!r.ok) throw new Error('HTTP ' + r.status);
            return r.json();
        })
        .then(function(data){
            // 概览卡片数值
            document.getElementById('stat-total-failed').textContent = data.total_failed || 0;
            document.getElementById('stat-auto-fixed').textContent = data.auto_fixed || 0;
            document.getElementById('stat-success-rate').textContent = data.success_rate || 0;
            document.getElementById('stat-avg-retries').textContent = data.avg_retries || 0;

            // 重试分布柱状图
            var dist = data.retry_distribution || [];
            chart.setOption({
                tooltip: {trigger: 'axis'},
                xAxis: {
                    type: 'category',
                    data: dist.map(function(d){ return '重试 ' + d.attempt + ' 次'; })
                },
                yAxis: {type: 'value'},
                series: [{
                    name: '任务数',
                    type: 'bar',
                    data: dist.map(function(d){ return d.count; }),
                    itemStyle: {color: '#1E9FFF'}
                }]
            });

            // 失败任务表格
            table.render({
                elem: '#failed-table',
                data: data.failed_tasks || [],
                page: true,
                limit: 20,
                cols: [[
                    {field: 'task_id', title: '任务 ID', width: 140, templet: function(d){
                        return '<a href="/tasks/' + d.task_id + '">' + d.task_id.substring(0, 8) + '</a>';
                    }},
                    {field: 'project', title: '项目', width: 200},
                    {field: 'task_description', title: '任务描述'},
                    {field: 'ci_retries', title: '重试次数', width: 100},
                    {field: 'error', title: '错误', templet: function(d){
                        return d.error ? '<span style="color:red;">' + d.error + '</span>' : '-';
                    }}
                ]]
            });
        })
        .catch(function(err){
            layui.layer.msg('加载 CI 统计失败: ' + err.message, {icon: 2});
        });

    window.addEventListener('resize', function(){
        chart.resize();
    });
});
</script>
{% endblock %}
```

- [ ] **Step 2: 在 routes.rs 添加 ci_page handler**

在 `routes.rs` 追加：

```rust
/// GET /ci - CI 自愈统计页
pub async fn ci_page() -> Result<Html<String>, DashboardError> {
    let tmpl = CiTemplate {
        active_nav: "ci".to_string(),
    };
    let html = tmpl.render()?;
    Ok(Html(html))
}
```

- [ ] **Step 3: 更新 routes.rs 顶部 import，添加 CiTemplate**

将 import 扩展为：

```rust
use crate::server::views::{IndexTemplate, TaskDetailTemplate, RealtimeTemplate, TrendsTemplate, CostTemplate, CiTemplate};
```

- [ ] **Step 4: 在 router 中注册 GET /ci 路由**

在 `router()` 函数中添加：

```rust
.route("/ci", get(ci_page))
```

- [ ] **Step 5: 验证编译**

Run: `cargo check -p devnpc-dashboard`
Expected: 编译通过。

- [ ] **Step 6: 提交 CI 自愈页**

Run: `git add crates/devnpc-dashboard/src/views/ci.html crates/devnpc-dashboard/src/server/routes.rs ; git commit -m "feat(dashboard): 实现 CI 自愈统计页 (ci.html + 概览卡片 + 重试分布柱状图)"`

---

### Task 10: SOP 偏离监控页（sop.html + handler + 路由）

**Files:**
- Create: `crates/devnpc-dashboard/src/views/sop.html`
- Modify: `crates/devnpc-dashboard/src/server/routes.rs`

**spec 引用:** §6.3.7 SOP 偏离监控 `/sop`

- [ ] **Step 1: 创建 src/views/sop.html**

```html
{% extends "layout.html" %}

{% block title %}SOP 偏离监控 - devnpc Dashboard{% endblock %}
{% block page_title %}SOP 偏离监控{% endblock %}

{% block content %}
<!-- ECharts 柱状图:按步骤统计偏离频率 -->
<div class="layui-row layui-col-space15">
    <div class="layui-col-md12">
        <div class="layui-card">
            <div class="layui-card-header">按步骤统计偏离频率</div>
            <div class="layui-card-body">
                <div id="chart-sop" style="height:300px;"></div>
            </div>
        </div>
    </div>
</div>

<!-- 偏离事件列表 -->
<div class="layui-row layui-col-space15">
    <div class="layui-col-md12">
        <div class="layui-card">
            <div class="layui-card-header">偏离事件列表</div>
            <div class="layui-card-body">
                <table id="deviation-table" lay-filter="deviation"></table>
            </div>
        </div>
    </div>
</div>
{% endblock %}

{% block scripts %}
<script src="/static/js/echarts.min.js"></script>
<script>
layui.use(['table'], function(){
    var table = layui.table;
    var chart = echarts.init(document.getElementById('chart-sop'));

    // 加载 SOP 偏离统计 (spec §5.2: GET /api/stats/sop)
    fetch('/api/stats/sop')
        .then(function(r){
            if (!r.ok) throw new Error('HTTP ' + r.status);
            return r.json();
        })
        .then(function(data){
            // 柱状图:步骤偏离频率
            var stepData = data.step_frequency || [];
            chart.setOption({
                tooltip: {trigger: 'axis'},
                xAxis: {
                    type: 'category',
                    data: stepData.map(function(d){ return d.step; })
                },
                yAxis: {type: 'value'},
                series: [{
                    name: '偏离次数',
                    type: 'bar',
                    data: stepData.map(function(d){ return d.count; }),
                    itemStyle: {color: '#FF5722'}
                }]
            });

            // 偏离事件列表表格
            table.render({
                elem: '#deviation-table',
                data: data.deviations || [],
                page: true,
                limit: 20,
                cols: [[
                    {field: 'created_at', title: '时间', width: 200},
                    {field: 'task_id', title: '任务', width: 140, templet: function(d){
                        return '<a href="/tasks/' + d.task_id + '">' + d.task_id.substring(0, 8) + '</a>';
                    }},
                    {field: 'step', title: '步骤', width: 200},
                    {field: 'note', title: '说明'}
                ]]
            });
        })
        .catch(function(err){
            layui.layer.msg('加载 SOP 偏离数据失败: ' + err.message, {icon: 2});
        });

    window.addEventListener('resize', function(){
        chart.resize();
    });
});
</script>
{% endblock %}
```

- [ ] **Step 2: 在 routes.rs 添加 sop_page handler**

在 `routes.rs` 追加：

```rust
/// GET /sop - SOP 偏离监控页
pub async fn sop_page() -> Result<Html<String>, DashboardError> {
    let tmpl = SopTemplate {
        active_nav: "sop".to_string(),
    };
    let html = tmpl.render()?;
    Ok(Html(html))
}
```

- [ ] **Step 3: 更新 routes.rs 顶部 import，添加 SopTemplate**

将 import 扩展为最终完整列表：

```rust
use crate::server::views::{
    IndexTemplate, TaskDetailTemplate, RealtimeTemplate,
    TrendsTemplate, CostTemplate, CiTemplate, SopTemplate,
};
```

- [ ] **Step 4: 在 router 中注册 GET /sop 路由**

在 `router()` 函数中添加：

```rust
.route("/sop", get(sop_page))
```

- [ ] **Step 5: 验证编译**

Run: `cargo check -p devnpc-dashboard`
Expected: 编译通过。

- [ ] **Step 6: 提交 SOP 偏离页**

Run: `git add crates/devnpc-dashboard/src/views/sop.html crates/devnpc-dashboard/src/server/routes.rs ; git commit -m "feat(dashboard): 实现 SOP 偏离监控页 (sop.html + 步骤频率柱状图 + 偏离事件列表)"`

---

### Task 11: 静态资源 /static/* 路由 + 完整启动验证

**Files:**
- Modify: `crates/devnpc-dashboard/src/server/routes.rs`

**spec 引用:** §5.2 路由表 `GET /static/*`

- [ ] **Step 1: 在 routes.rs 添加 static_handler 处理通配路径**

在 `routes.rs` 追加（处理 `/static/*path` 通配路由，将路径转发给 Task 2 的 `serve_static`）：

```rust
use axum::extract::Path as AxumPath;

/// GET /static/*path - 静态资源 (rust-embed 嵌入)
///
/// path 形如 "layui/layui.js" / "css/dashboard.css" / "js/echarts.min.js"
pub async fn static_handler(AxumPath(path): AxumPath<String>) -> Response {
    // path 可能含子路径分隔符 (如 "layui/css/layui.css"),直接传递给 serve_static
    serve_static(&path)
}
```

注意：`axum::extract::Path<String>` 用于单段通配。如果路径含多段（如 `layui/css/layui.css`），axum 默认会按 `/` 分割。要捕获多段路径，有两种方式：

方式 A（推荐，使用 `*path` 通配）: 在路由定义为 `/static/*path`，handler 使用 `Path<String>` 接收完整剩余路径（axum 会把 `layui/css/layui.css` 作为单个 String 传入）。

方式 B: 使用 `Path<Vec<String>>` 接收多段，然后 join。

本计划采用方式 A。axum 0.7 中 `/static/*path` 路由的 `Path<String>` 参数会接收去掉 `/static/` 前缀后的完整剩余路径（含子目录）。

如果 axum 实际行为是把多段路径作为单个 String 传入时去掉了 `/`，则改用方式 B：

```rust
pub async fn static_handler(AxumPath(parts): AxumPath<Vec<String>>) -> Response {
    let path = parts.join("/");
    serve_static(&path)
}
```

根据 axum 0.7 文档，`/static/*path` + `Path<String>` 会接收形如 `layui/css/layui.css` 的完整路径（保留 `/`）。优先使用 `Path<String>` 版本，如运行时发现路径被截断，再切换为 `Vec<String>` 版本。

- [ ] **Step 2: 更新 routes.rs 顶部 import**

在 `routes.rs` 顶部确认有：

```rust
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
```

如使用 `Path<String>`，无需额外 import（`Path` 已导入）。如改用 `Path<Vec<String>>`，同样无需修改 import（`Path` 是同一个类型，泛型参数不同）。

- [ ] **Step 3: 在 router 中注册 GET /static/*path 路由**

在 `router()` 函数中，**在所有具体页面路由之后、API 路由之前**添加（避免与 `/static` 冲突的顺序问题）：

```rust
.route("/static/*path", get(static_handler))
```

完整 router 函数示例（阶段 4 完成后的最终形态）：

```rust
pub fn router(state: AppState) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        // === 页面路由 (Phase 4) ===
        .route("/", get(index_page))
        .route("/tasks/:id", get(task_detail_page))
        .route("/realtime", get(realtime_page))
        .route("/trends", get(trends_page))
        .route("/cost", get(cost_page))
        .route("/ci", get(ci_page))
        .route("/sop", get(sop_page))
        // === 静态资源 (Phase 4) ===
        .route("/static/*path", get(static_handler))
        // === 推送 API (阶段 3,需 token) ===
        .route("/api/events/start", post(crate::server::api::events_start))
        .route("/api/events/batch", post(crate::server::api::events_batch))
        .route("/api/events/finish", post(crate::server::api::events_finish))
        .route("/api/events/import", post(crate::server::api::events_import))
        // === 查询 API (阶段 3,免鉴权) ===
        .route("/api/tasks", get(crate::server::api::list_tasks))
        .route("/api/tasks/:id", get(crate::server::api::get_task))
        .route("/api/tasks/:id/events", get(crate::server::api::list_events))
        .route("/api/realtime/stream", get(crate::server::api::realtime_stream))
        .route("/api/stats/trends", get(crate::server::api::stats_trends))
        .route("/api/stats/cost", get(crate::server::api::stats_cost))
        .route("/api/stats/ci", get(crate::server::api::stats_ci))
        .route("/api/stats/sop", get(crate::server::api::stats_sop))
        .with_state(state)
}
```

注意：阶段 3 的具体 handler 函数名可能略有不同，按实际名称调整。如果阶段 3 把推送 API 路由单独放在 `api::router()` 中并应用了 token 中间件，则保持阶段 3 的拆分方式，本 Task 只追加页面路由和静态资源路由。

- [ ] **Step 4: 验证完整编译**

Run: `cargo check -p devnpc-dashboard`
Expected: 编译通过，无 warning（如有 unused import，移除）。

Run: `cargo build -p devnpc-dashboard`
Expected: 构建成功。

- [ ] **Step 5: 启动服务并验证所有页面可访问**

Run: `.\target\debug\devnpc-dashboard.exe --port 18080 --db .\test-dashboard.db --token test-token-123`

新开终端，逐个访问 7 个页面（应返回 HTML，即使 LayUI/ECharts 静态文件未放入也不会让页面 500，只是样式缺失）：

Run: `Invoke-WebRequest -Uri http://localhost:18080/ -UseBasicParsing | Select-Object StatusCode`
Expected: `200`

Run: `Invoke-WebRequest -Uri http://localhost:18080/realtime -UseBasicParsing | Select-Object StatusCode`
Expected: `200`

Run: `Invoke-WebRequest -Uri http://localhost:18080/trends -UseBasicParsing | Select-Object StatusCode`
Expected: `200`

Run: `Invoke-WebRequest -Uri http://localhost:18080/cost -UseBasicParsing | Select-Object StatusCode`
Expected: `200`

Run: `Invoke-WebRequest -Uri http://localhost:18080/ci -UseBasicParsing | Select-Object StatusCode`
Expected: `200`

Run: `Invoke-WebRequest -Uri http://localhost:18080/sop -UseBasicParsing | Select-Object StatusCode`
Expected: `200`

验证任务详情页（即使任务不存在，应返回 404 而非 500）：

Run: `Invoke-WebRequest -Uri http://localhost:18080/tasks/nonexistent-id -UseBasicParsing | Select-Object StatusCode`
Expected: `404`

验证静态资源（dashboard.css 已在 Task 1 创建，应返回 200 + `text/css`）：

Run: `(Invoke-WebRequest -Uri http://localhost:18080/static/css/dashboard.css -UseBasicParsing).Headers["Content-Type"]`
Expected: `text/css`

停止服务: `Ctrl+C`。

- [ ] **Step 6: 提交静态资源路由**

Run: `git add crates/devnpc-dashboard/src/server/routes.rs ; git commit -m "feat(dashboard): 添加 /static/*path 静态资源路由,完成 7 个页面 + 静态资源完整集成"`

---

### Task 12: 视图层集成测试

**Files:**
- Create: `crates/devnpc-dashboard/tests/view_handlers.rs`

**spec 引用:** §8.2 集成测试

- [ ] **Step 1: 创建 tests/view_handlers.rs**

```rust
//! 视图层 handler 集成测试
//!
//! 验证 7 个页面 handler 在空数据库和有数据情况下均能正确返回 HTML。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use devnpc_dashboard::server::routes::router;
use devnpc_dashboard::storage::Storage;
use devnpc_dashboard::AppState;

/// 构造测试用 AppState (临时 SQLite 文件)
async fn make_state() -> AppState {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    AppState {
        storage: std::sync::Arc::new(storage),
        realtime: std::sync::Arc::new(devnpc_dashboard::realtime::RealtimeHub::new(1000)),
        config: std::sync::Arc::new(devnpc_dashboard::DashboardConfig {
            token: "test-token".to_string(),
        }),
    }
}

#[tokio::test]
async fn index_page_returns_html_with_title() {
    let state = make_state().await;
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("任务列表"), "应包含页面标题");
    assert!(html.contains("layui-layout-admin"), "应包含 LayUI 布局类");
    assert!(html.contains("导入事件文件"), "应包含导入按钮");
}

#[tokio::test]
async fn realtime_page_returns_html() {
    let state = make_state().await;
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri("/realtime").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("EventSource"), "应包含 SSE EventSource 代码");
}

#[tokio::test]
async fn trends_page_returns_html_with_charts() {
    let state = make_state().await;
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri("/trends").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("chart-success"), "应包含成功率图表容器");
    assert!(html.contains("echarts.init"), "应包含 ECharts 初始化代码");
}

#[tokio::test]
async fn cost_page_returns_html() {
    let state = make_state().await;
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri("/cost").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("chart-pie"), "应包含饼图容器");
    assert!(html.contains("group_by"), "应包含分组维度切换");
}

#[tokio::test]
async fn ci_page_returns_html() {
    let state = make_state().await;
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri("/ci").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("stat-total-failed"), "应包含总失败统计卡片");
    assert!(html.contains("chart-retry"), "应包含重试分布图表");
}

#[tokio::test]
async fn sop_page_returns_html() {
    let state = make_state().await;
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri("/sop").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("chart-sop"), "应包含 SOP 图表");
    assert!(html.contains("deviation-table"), "应包含偏离事件列表表格");
}

#[tokio::test]
async fn task_detail_page_returns_404_for_unknown_task() {
    let state = make_state().await;
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/tasks/nonexistent-uuid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn static_handler_serves_dashboard_css() {
    let state = make_state().await;
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/static/css/dashboard.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp.headers().get("content-type").unwrap();
    assert_eq!(ct, "text/css");
}

#[tokio::test]
async fn static_handler_returns_404_for_missing_asset() {
    let state = make_state().await;
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/static/nonexistent/file.xyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
```

**注意:** 上述测试假设 `devnpc-dashboard` crate 有 `lib.rs` 暴露 `pub mod server` / `pub mod storage` / `pub mod realtime` 以及 `pub struct AppState` / `pub struct DashboardConfig`。如果阶段 3 把这些类型放在 `main.rs` 中（即没有 lib target），需要：

1. 在 `crates/devnpc-dashboard/Cargo.toml` 中确认有 `[lib]` 段：

```toml
[lib]
name = "devnpc_dashboard"
path = "src/lib.rs"
```

2. 创建 `crates/devnpc-dashboard/src/lib.rs` 暴露公共 API：

```rust
pub mod auth;
pub mod error;
pub mod realtime;
pub mod server;
pub mod static_files;
pub mod storage;

pub use server::AppState;
pub use server::DashboardConfig;
```

3. `main.rs` 改为 `use devnpc_dashboard::*;` 调用 lib 中的逻辑。

如果阶段 3 已是 lib + bin 结构，则跳过此调整。

- [ ] **Step 2: 验证 lib 目标存在**

Run: `cargo check -p devnpc-dashboard --lib`
Expected: 编译通过。如报错 `no lib target`，按 Step 1 末尾的说明添加 `[lib]` 段和 `src/lib.rs`。

- [ ] **Step 3: 运行视图层测试**

Run: `cargo test -p devnpc-dashboard --test view_handlers`
Expected: 9 个测试全部 PASS

- [ ] **Step 4: 运行整个 dashboard crate 的测试套件**

Run: `cargo test -p devnpc-dashboard`
Expected: 阶段 3 的所有测试 + Task 2 的 static_files 测试 + Task 12 的 9 个视图测试，全部 PASS。

- [ ] **Step 5: 运行 clippy**

Run: `cargo clippy -p devnpc-dashboard -- -D warnings`
Expected: 无 warning。如有 `unused_imports`，移除 routes.rs / views.rs 中未使用的 import。

- [ ] **Step 6: 提交视图层测试**

Run: `git add crates/devnpc-dashboard/tests/view_handlers.rs crates/devnpc-dashboard/src/lib.rs crates/devnpc-dashboard/Cargo.toml ; git commit -m "test(dashboard): 添加视图层 9 个 handler 集成测试 (页面渲染 + 静态资源 + 404)"`

---

## Self-Review 检查清单

### 1. spec 覆盖检查

- [x] **spec §5.6 视图层**: askama 编译期模板 + layout.html 公共布局 + 7 个页面模板 — Task 3 (layout) + Task 4-10 (7 个页面)
- [x] **spec §5.2 页面路由**: GET / / /tasks/:id / /realtime / /trends / /cost / /ci / /sop / /static/* — Task 4-11 全部覆盖
- [x] **spec §6.1 技术栈**: LayUI 2.x (用户后续放入) + ECharts + askama SSR + EventSource — 全部体现
- [x] **spec §6.2 页面结构**: 顶栏 + 侧边栏导航 + 主内容区 — Task 3 layout.html 实现
- [x] **spec §6.3.1 任务列表**: layui-table + AJAX /api/tasks + 状态 badge + 5 秒自动刷新 + 导入按钮 — Task 4 实现
- [x] **spec §6.3.2 任务详情**: layui-timeline + 不同事件类型不同图标 + 元信息卡片 — Task 5 实现
- [x] **spec §6.3.3 实时监控**: SSE EventSource + layui-collapse 折叠面板 + 完成变色 3 秒收起 — Task 6 实现
- [x] **spec §6.3.4 趋势统计**: 4 个 ECharts (成功率/耗时/Token/成本) + 7/30/90 天切换 — Task 7 实现
- [x] **spec §6.3.5 成本分析**: ECharts 饼图 + 明细表格 + 分组维度切换 — Task 8 实现
- [x] **spec §6.3.6 CI 自愈**: 概览卡片 + ECharts 柱状图 + 失败任务列表 — Task 9 实现
- [x] **spec §6.3.7 SOP 偏离**: ECharts 柱状图 + 偏离事件列表 — Task 10 实现
- [x] **spec §6.4 实时刷新策略**: 任务列表 5 秒（仅 running 时）/ 实时监控 SSE / 其他不刷新 — Task 4 (done 回调) + Task 6 实现
- [x] **spec §6.5 错误处理**: AJAX 失败 layer.msg + SSE 断连自动重连 — Task 4/6 JS 代码体现
- [x] **spec §3.4 导入按钮**: 任务列表页顶部"导入事件文件"按钮 + layui-upload — Task 4 index.html 实现
- [x] **rust-embed 嵌入 static/**: Task 1 (目录占位) + Task 2 (static_files.rs) + Task 11 (路由)
- [x] **依赖添加**: askama + rust-embed + mime_guess — Task 1

### 2. 占位符扫描

- 已检查所有 Task 的代码步骤，均包含完整代码（HTML/Rust/JS），无 "TBD"/"TODO"/"实现错误处理" 等占位符。
- 路由函数示例中提到的"阶段 3 已有 API handler 函数名"按实际名称调整的说明，属于合理的对接说明而非占位符。

### 3. 类型一致性

- `IndexTemplate` / `TaskDetailTemplate` / `RealtimeTemplate` / `TrendsTemplate` / `CostTemplate` / `CiTemplate` / `SopTemplate`：Task 3 定义，Task 4-10 在 handler 中使用，字段名 `active_nav` 一致。
- `TaskRow`：Task 5 handler 使用 `state.storage.get_task(&task_id)?` 返回 `Result<Option<TaskRow>>`，与 spec §5.5 一致；模板中访问 `task.task_id` / `task.status` / `task.finished_at` 等字段，与"前置条件"中列出的字段假设一致。
- `DashboardError::TaskNotFound`：Task 5 handler 使用，与 spec §7.3 定义一致。
- `serve_static(path: &str) -> Response`：Task 2 定义，Task 11 的 `static_handler` 调用，签名一致。
- `AppState`：Task 5 handler 通过 `State(state): State<AppState>` 提取，与阶段 3 假设一致。

### 4. 完整性

- 7 个页面模板 + layout.html 公共布局 + 7 个 handler + 9 条路由（含 /static/*）全部覆盖。
- rust-embed 嵌入逻辑 + 静态资源路由完整。
- 视图层测试覆盖 9 个场景（含 404 / 静态资源 / 各页面 HTML 内容校验）。
- 每个 Task 末尾均有 commit 步骤。
- 所有命令使用 Windows PowerShell 语法（`;` 分隔、反斜杠路径）。

---

## 执行说明

**前置依赖:** 阶段 1-3 必须已完成（workspace 拆分、core 类型、dashboard 存储 + 服务端 API）。

**与用户协作点:**
1. Task 1 创建的 `static/` 目录占位仅含 `dashboard.css`。用户需在 Task 11 验证启动前下载并放入：
   - LayUI 2.x：解压到 `crates/devnpc-dashboard/static/layui/`（确保 `static/layui/layui.js` 和 `static/layui/css/layui.css` 存在）
   - ECharts：下载 `echarts.min.js` 放到 `crates/devnpc-dashboard/static/js/echarts.min.js`
2. 即使未放入 LayUI/ECharts，Task 11 Step 5 的页面访问仍应返回 200（只是页面样式缺失，JS 报 404 不影响 SSR HTML 渲染）。

**回退策略:** 若 askama 编译期模板配置 `template_dirs` 在某些 askama 版本下不生效，可改用 `#[template(path = "src/views/index.html")]` 绝对路径方式（每个 struct 显式指定完整相对路径）。
