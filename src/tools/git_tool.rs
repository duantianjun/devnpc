//! Git 工具 (P3 实现: git_commit, git_diff)

use std::path::PathBuf;

pub struct GitTool {
    #[allow(dead_code)]
    pub workspace: PathBuf,
}

impl GitTool {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }
}
