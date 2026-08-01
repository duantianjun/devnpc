//! AFT 文件操作工具 (P3 实现: view_symbol, edit_symbol, ast_replace, outline, search_symbols)
//!
//! 基于 agent-file-tools (tree-sitter) 实现符号级读改。

use std::path::PathBuf;
use crate::error::{DevnpcError, Result};

pub struct FileIo {
    pub workspace: PathBuf,
}

impl FileIo {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }

    /// 路径安全检查 (防 path traversal)
    ///
    /// 不依赖 canonicalize (文件可能不存在),用 components 检查路径是否逃出 workspace。
    pub fn validate_path(&self, path: &str) -> Result<PathBuf> {
        let full = self.workspace.join(path);
        // 用 components 检查: 若路径中 .. 导致跳出 workspace,则拒绝
        let mut depth: i32 = 0;
        for comp in std::path::Path::new(path).components() {
            match comp {
                std::path::Component::ParentDir => depth -= 1,
                std::path::Component::Normal(_) => depth += 1,
                std::path::Component::RootDir => {
                    // 绝对路径,直接拒绝
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn validate_path_blocks_traversal() {
        let dir = tempdir().unwrap();
        let file_io = FileIo::new(dir.path());
        let result = file_io.validate_path("../etc/passwd");
        assert!(matches!(result, Err(DevnpcError::PathTraversal { .. })));
    }

    #[test]
    fn validate_path_allows_inside_workspace() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("src");
        fs::create_dir_all(&sub).unwrap();
        let file_io = FileIo::new(dir.path());
        let result = file_io.validate_path("src/main.rs");
        // 不报 PathTraversal 错误即通过 (文件可能不存在,canonicalize 会失败)
        assert!(!matches!(result, Err(DevnpcError::PathTraversal { .. })));
    }
}
