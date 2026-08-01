//! 自建文件工具: read_file, write_file, list_files
//!
//! 全部限制在 workspace 内 (复用 validate_path 防 path traversal)。

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{DevnpcError, Result};
use crate::tools::{Tool, ToolResult};

/// 文件工具共享的 workspace 上下文
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

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
}

pub struct ReadFileTool {
    file_io: FileIo,
}

impl ReadFileTool {
    pub fn new(file_io: FileIo) -> Self {
        Self { file_io }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "读取 workspace 内文件全文 (限前 200 行)。path 相对 workspace 根。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"path": {"type": "string", "description": "相对 workspace 的文件路径"}},
            "required": ["path"]
        })
    }
    async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let args: ReadFileArgs = serde_json::from_value(args.clone())
            .map_err(|e| DevnpcError::Tool {
                tool: "read_file".into(),
                msg: format!("参数解析失败: {e}"),
            })?;
        let full = self.file_io.validate_path(&args.path)?;
        let content = match std::fs::read_to_string(&full) {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::err(format!("读取失败: {e}")));
            }
        };
        // 限 200 行防 token 爆炸
        let truncated: String = content.lines().take(200).collect::<Vec<_>>().join("\n");
        Ok(ToolResult::ok(truncated))
    }
}

#[derive(Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
}

pub struct WriteFileTool {
    file_io: FileIo,
}

impl WriteFileTool {
    pub fn new(file_io: FileIo) -> Self {
        Self { file_io }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "写入 workspace 内文件 (全量覆盖)。path 相对 workspace 根。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string", "description": "完整文件内容"}
            },
            "required": ["path", "content"]
        })
    }
    async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let args: WriteFileArgs = serde_json::from_value(args.clone())
            .map_err(|e| DevnpcError::Tool {
                tool: "write_file".into(),
                msg: format!("参数解析失败: {e}"),
            })?;
        let full = self.file_io.validate_path(&args.path)?;
        // 确保父目录存在
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match std::fs::write(&full, &args.content) {
            Ok(_) => Ok(ToolResult::ok(format!("已写入 {}", args.path))),
            Err(e) => Ok(ToolResult::err(format!("写入失败: {e}"))),
        }
    }
}

pub struct ListFilesTool {
    file_io: FileIo,
}

impl ListFilesTool {
    pub fn new(file_io: FileIo) -> Self {
        Self { file_io }
    }
}

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &str {
        "list_files"
    }
    fn description(&self) -> &str {
        "列出 workspace 内指定目录的条目 (文件/子目录名)。dir 相对 workspace 根,默认 \"\"。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"dir": {"type": "string", "default": ""}}
        })
    }
    async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let dir = args["dir"].as_str().unwrap_or("");
        let full = self.file_io.validate_path(dir)?;
        if !full.is_dir() {
            return Ok(ToolResult::err(format!("不是目录: {dir}")));
        }
        let mut entries: Vec<String> = std::fs::read_dir(&full)
            .map_err(|e| DevnpcError::Tool {
                tool: "list_files".into(),
                msg: format!("读取目录失败: {e}"),
            })?
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if e.path().is_dir() {
                    format!("{name}/")
                } else {
                    name
                }
            })
            .collect();
        entries.sort();
        Ok(ToolResult::ok(entries.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn read_file_returns_content() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\nworld").unwrap();
        let tool = ReadFileTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"path": "a.txt"}))
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output, "hello\nworld");
    }

    #[tokio::test]
    async fn read_file_truncates_at_200_lines() {
        let dir = tempdir().unwrap();
        let content = (1..=300).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        std::fs::write(dir.path().join("big.txt"), content).unwrap();
        let tool = ReadFileTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"path": "big.txt"}))
            .await
            .unwrap();
        let lines = result.output.lines().count();
        assert_eq!(lines, 200);
    }

    #[tokio::test]
    async fn read_file_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let tool = ReadFileTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"path": "../etc/passwd"}))
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DevnpcError::PathTraversal { .. }));
    }

    #[tokio::test]
    async fn read_file_returns_err_for_missing_file() {
        let dir = tempdir().unwrap();
        let tool = ReadFileTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"path": "nope.txt"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("读取失败"));
    }

    #[tokio::test]
    async fn write_file_creates_new_file() {
        let dir = tempdir().unwrap();
        let tool = WriteFileTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"path": "out.txt", "content": "data"}))
            .await
            .unwrap();
        assert!(result.success);
        let written = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
        assert_eq!(written, "data");
    }

    #[tokio::test]
    async fn write_file_creates_parent_dirs() {
        let dir = tempdir().unwrap();
        let tool = WriteFileTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"path": "src/handler/login.rs", "content": "fn login() {}"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(dir.path().join("src/handler/login.rs").exists());
    }

    #[tokio::test]
    async fn list_files_returns_sorted_entries() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("b.txt"), "b").unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        let tool = ListFilesTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"dir": ""}))
            .await
            .unwrap();
        assert!(result.success);
        let entries: Vec<&str> = result.output.lines().collect();
        assert!(entries.contains(&"a.txt"));
        assert!(entries.contains(&"b.txt"));
        assert!(entries.contains(&"sub/"));
    }

    #[tokio::test]
    async fn list_files_err_for_non_dir() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), "x").unwrap();
        let tool = ListFilesTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"dir": "file.txt"}))
            .await
            .unwrap();
        assert!(!result.success);
    }
}
