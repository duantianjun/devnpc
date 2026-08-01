//! Shell 命令工具 (P3 实现: run_command)
//!
//! 沙箱内执行,带白名单/黑名单 + 超时。

use std::path::PathBuf;

pub struct Shell {
    #[allow(dead_code)]
    pub workspace: PathBuf,
}

impl Shell {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }
}
