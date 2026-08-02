//! 文件 I/O 工具: 带路径安全检查的 workspace 文件操作
//!
//! 从旧 tools/file_io.rs 迁移而来,移除 Tool trait 依赖,保留 FileIo 核心结构体。

use std::path::PathBuf;

use crate::error::{DevnpcError, Result};

/// 文件工具共享的 workspace 上下文
#[derive(Clone)]
pub struct FileIo {
    pub workspace: PathBuf,
}

impl FileIo {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    /// 路径安全检查 (防 path traversal)
    pub fn validate_path(&self, path: &str) -> Result<PathBuf> {
        let full = self.workspace.join(path);
        let mut depth: i32 = 0;
        for comp in std::path::Path::new(path).components() {
            match comp {
                std::path::Component::ParentDir => depth -= 1,
                std::path::Component::Normal(_) => depth += 1,
                std::path::Component::RootDir => {
                    return Err(DevnpcError::PathTraversal { path: path.into() });
                }
                _ => {}
            }
            if depth < 0 {
                return Err(DevnpcError::PathTraversal { path: path.into() });
            }
        }
        Ok(full)
    }
}