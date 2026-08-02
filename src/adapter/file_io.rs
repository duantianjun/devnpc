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

    /// 路径安全检查 (防 path traversal + symlink 绕过)
    ///
    /// 两层防护:
    /// 1. 组件检查: 拒绝 `..` 越界和绝对路径 (拦截 `../` / `/etc/passwd` / `C:\`)
    /// 2. Canonicalize 前缀校验: 解析符号链接后,验证最终路径仍在 workspace 内
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
                #[cfg(windows)]
                std::path::Component::Prefix(_) => {
                    // Windows UNC/盘符前缀 (如 \\?\C:\, C:\)
                    return Err(DevnpcError::PathTraversal { path: path.into() });
                }
                _ => {}
            }
            if depth < 0 {
                return Err(DevnpcError::PathTraversal { path: path.into() });
            }
        }
        // Canonicalize 校验: 解析 symlink 后确认仍在 workspace 内
        // 若目标文件尚不存在 (写新文件场景),canonicalize 会失败,此时对 parent 校验
        let ws_canon = self.workspace.canonicalize().unwrap_or_else(|_| self.workspace.clone());
        match full.canonicalize() {
            Ok(canon) => {
                if !canon.starts_with(&ws_canon) {
                    return Err(DevnpcError::PathTraversal { path: path.into() });
                }
                Ok(canon)
            }
            Err(_) => {
                // 目标不存在 (写新文件): 校验 parent 目录
                if let Some(parent) = full.parent()
                    && let Ok(parent_canon) = parent.canonicalize()
                    && !parent_canon.starts_with(&ws_canon)
                {
                    return Err(DevnpcError::PathTraversal { path: path.into() });
                }
                Ok(full)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// 创建临时 workspace 并写入若干文件,返回 TempDir (必须保持存活)
    fn setup_workspace() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "# test\n").unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        dir
    }

    #[test]
    fn validate_path_accepts_normal_relative_path() {
        let dir = setup_workspace();
        let file_io = FileIo::new(dir.path());
        let result = file_io.validate_path("README.md");
        assert!(result.is_ok(), "正常相对路径应通过: {:?}", result.err());
    }

    #[test]
    fn validate_path_accepts_subdir_path() {
        let dir = setup_workspace();
        let file_io = FileIo::new(dir.path());
        let result = file_io.validate_path("src/main.rs");
        assert!(result.is_ok(), "子目录路径应通过: {:?}", result.err());
    }

    #[test]
    fn validate_path_rejects_parent_dir_traversal() {
        let dir = setup_workspace();
        let file_io = FileIo::new(dir.path());
        let result = file_io.validate_path("../etc/passwd");
        let err = result.unwrap_err();
        assert!(matches!(err, DevnpcError::PathTraversal { .. }), "实际: {err}");
    }

    #[test]
    fn validate_path_rejects_double_parent_dir_escape() {
        let dir = setup_workspace();
        let file_io = FileIo::new(dir.path());
        // a/../.. 会让 depth 从 1 → 0 → -1,触发越界
        let result = file_io.validate_path("a/../..");
        let err = result.unwrap_err();
        assert!(matches!(err, DevnpcError::PathTraversal { .. }), "实际: {err}");
    }

    #[test]
    fn validate_path_rejects_balanced_but_escaping_path() {
        let dir = setup_workspace();
        let file_io = FileIo::new(dir.path());
        // src/../../etc: 第一段 src 抵消一个 ..,但仍有 .. 越界
        let result = file_io.validate_path("src/../../etc/passwd");
        let err = result.unwrap_err();
        assert!(matches!(err, DevnpcError::PathTraversal { .. }), "实际: {err}");
    }

    #[test]
    fn validate_path_rejects_absolute_unix_path() {
        let dir = setup_workspace();
        let file_io = FileIo::new(dir.path());
        let result = file_io.validate_path("/etc/passwd");
        let err = result.unwrap_err();
        assert!(matches!(err, DevnpcError::PathTraversal { .. }), "实际: {err}");
    }

    #[cfg(windows)]
    #[test]
    fn validate_path_rejects_windows_drive_prefix() {
        let dir = setup_workspace();
        let file_io = FileIo::new(dir.path());
        let result = file_io.validate_path("C:\\Windows\\System32");
        let err = result.unwrap_err();
        assert!(matches!(err, DevnpcError::PathTraversal { .. }), "实际: {err}");
    }

    #[cfg(windows)]
    #[test]
    fn validate_path_rejects_unc_path() {
        let dir = setup_workspace();
        let file_io = FileIo::new(dir.path());
        let result = file_io.validate_path("\\\\?\\C:\\Windows");
        let err = result.unwrap_err();
        assert!(matches!(err, DevnpcError::PathTraversal { .. }), "实际: {err}");
    }

    #[test]
    fn validate_path_allows_new_file_in_existing_subdir() {
        let dir = setup_workspace();
        let file_io = FileIo::new(dir.path());
        // 写新文件: 目标不存在,但 parent (workspace/src) 存在且在 workspace 内
        let result = file_io.validate_path("src/new_file.rs");
        assert!(result.is_ok(), "已存在子目录中的新文件应通过: {:?}", result.err());
    }

    #[test]
    fn validate_path_allows_new_file_in_nested_missing_subdir() {
        let dir = setup_workspace();
        let file_io = FileIo::new(dir.path());
        // workspace/a/b/c.txt: parent (workspace/a/b) 不存在,canonicalize 失败,不触发拦截
        let result = file_io.validate_path("a/b/c.txt");
        assert!(result.is_ok(), "深层不存在子目录新文件应通过: {:?}", result.err());
    }

    #[test]
    fn validate_path_rejects_parent_dir_when_new_file_parent_is_outside() {
        let dir = setup_workspace();
        let file_io = FileIo::new(dir.path());
        // 通过 symlink 测试较为可靠,这里测试相对路径越界到外部目录
        // 路径: ../outside.txt → full.parent() = workspace 的 parent (存在且在 workspace 外)
        // 但 ../outside.txt 已在组件检查阶段被拦截 (depth < 0)
        let result = file_io.validate_path("../outside.txt");
        let err = result.unwrap_err();
        assert!(matches!(err, DevnpcError::PathTraversal { .. }), "实际: {err}");
    }

    #[cfg(unix)]
    #[test]
    fn validate_path_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let dir = setup_workspace();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        symlink(outside.path(), dir.path().join("escape")).unwrap();
        let file_io = FileIo::new(dir.path());
        // 组件检查通过 (escape/secret.txt 不含 ..),但 canonicalize 校验应拦截
        let result = file_io.validate_path("escape/secret.txt");
        let err = result.unwrap_err();
        assert!(matches!(err, DevnpcError::PathTraversal { .. }), "实际: {err}");
    }

    #[test]
    fn file_io_clone_preserves_workspace() {
        let dir = setup_workspace();
        let file_io = FileIo::new(dir.path());
        let cloned = file_io.clone();
        assert_eq!(file_io.workspace, cloned.workspace);
    }
}