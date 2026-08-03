//! askama 模板对应的 Rust struct 定义
//!
//! 每个页面一个 struct,通过 `#[derive(Template)]` 关联 HTML 模板。
//! 模板文件位于 `src/views/`,通过 crate 根目录的 `askama.toml` 配置
//! (`dirs = ["src/views"]`)。子模板通过 `{% extends "layout.html" %}`
//! 继承公共布局,布局中通过 `active_nav` 字段高亮当前导航项。
