//! Git 工具: git_diff, git_commit

use std::path::PathBuf;
use std::process::Command;

use async_trait::async_trait;

use crate::error::{DevnpcError, Result};
use crate::git::ops::GitOps;
use crate::tools::{Tool, ToolResult};

pub struct GitDiffTool {
    workspace: PathBuf,
}

impl GitDiffTool {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }
}

#[async_trait]
impl Tool for GitDiffTool {
    fn name(&self) -> &str {
        "git_diff"
    }
    fn description(&self) -> &str {
        "查看当前工作区相对 HEAD 的未提交改动 (git diff HEAD)。无改动返回空字符串。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    async fn call(&self, _args: &serde_json::Value) -> Result<ToolResult> {
        let output = Command::new("git")
            .args(["diff", "HEAD"])
            .current_dir(&self.workspace)
            .output()
            .map_err(|e| DevnpcError::Tool {
                tool: "git_diff".into(),
                msg: format!("执行 git diff 失败: {e}"),
            })?;
        if !output.status.success() {
            return Ok(ToolResult::err("git diff 失败"));
        }
        let diff = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(ToolResult::ok(diff))
    }
}

pub struct GitCommitTool {
    ops: GitOps,
}

impl GitCommitTool {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            ops: GitOps::new(workspace),
        }
    }
}

#[async_trait]
impl Tool for GitCommitTool {
    fn name(&self) -> &str {
        "git_commit"
    }
    fn description(&self) -> &str {
        "提交当前所有改动 (git add -A + git commit)。参数: message (commit message)。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"message": {"type": "string"}},
            "required": ["message"]
        })
    }
    async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let message = args["message"].as_str().unwrap_or("");
        if message.is_empty() {
            return Ok(ToolResult::err("message 不能为空"));
        }
        match self.ops.commit(message).await {
            Ok(_) => Ok(ToolResult::ok(format!("已提交: {message}"))),
            Err(e) => Ok(ToolResult::err(format!("提交失败: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_repo() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        Command::new("git").args(["init"]).current_dir(&repo).output().unwrap();
        Command::new("git").args(["config", "user.email", "t@t.com"]).current_dir(&repo).output().unwrap();
        Command::new("git").args(["config", "user.name", "T"]).current_dir(&repo).output().unwrap();
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        Command::new("git").args(["add", "-A"]).current_dir(&repo).output().unwrap();
        Command::new("git").args(["commit", "-m", "init"]).current_dir(&repo).output().unwrap();
        (dir, repo)
    }

    #[tokio::test]
    async fn git_diff_returns_empty_when_no_changes() {
        let (_dir, repo) = setup_repo();
        let tool = GitDiffTool::new(&repo);
        let result = tool.call(&serde_json::json!({})).await.unwrap();
        assert!(result.success);
        assert!(result.output.is_empty());
    }

    #[tokio::test]
    async fn git_diff_returns_changes_after_modification() {
        let (_dir, repo) = setup_repo();
        std::fs::write(repo.join("a.txt"), "modified").unwrap();
        let tool = GitDiffTool::new(&repo);
        let result = tool.call(&serde_json::json!({})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("modified"));
    }

    #[tokio::test]
    async fn git_commit_creates_new_commit() {
        let (_dir, repo) = setup_repo();
        std::fs::write(repo.join("b.txt"), "b").unwrap();
        let tool = GitCommitTool::new(&repo);
        let result = tool
            .call(&serde_json::json!({"message": "add b"}))
            .await
            .unwrap();
        assert!(result.success);
        // 验证 commit 已创建
        let log = Command::new("git")
            .args(["log", "--oneline", "-1"])
            .current_dir(&repo)
            .output()
            .unwrap();
        let log_str = String::from_utf8_lossy(&log.stdout);
        assert!(log_str.contains("add b"));
    }

    #[tokio::test]
    async fn git_commit_rejects_empty_message() {
        let (_dir, repo) = setup_repo();
        let tool = GitCommitTool::new(&repo);
        let result = tool
            .call(&serde_json::json!({"message": ""}))
            .await
            .unwrap();
        assert!(!result.success);
    }
}
