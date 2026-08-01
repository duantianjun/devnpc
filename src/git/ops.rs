//! Git 命令封装 (P2 完整实现)
//!
//! 通过 std::process::Command 调用系统 git,避免 libgit2 C 依赖。
//! 同步执行: CI 单任务环境,git 命令通常较快;clone/push 较慢但可接受。

use std::path::PathBuf;
use std::process::Command;

use crate::error::{DevnpcError, Result};

/// Git 操作封装
pub struct GitOps {
    /// 工作目录
    pub workspace: PathBuf,
}

impl GitOps {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    /// 执行 git 命令,返回 stdout (trim 后)。
    /// 非 0 退出码返回 GitCommand 错误。
    fn run_git_cmd(&self, args: &[String]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.workspace)
            .output()
            .map_err(|_e| DevnpcError::GitCommand {
                cmd: format!("git {}", args.join(" ")),
                code: -1,
            })?;
        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            return Err(DevnpcError::GitCommand {
                cmd: format!("git {}", args.join(" ")),
                code,
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(stdout)
    }

    /// clone 仓库到 workspace 目录
    pub async fn clone_repo(&self, url: &str, branch: &str) -> Result<()> {
        // clone 到 workspace 自身 (workspace 必须不存在或为空)
        let args: Vec<String> = vec![
            "clone".into(),
            "--branch".into(),
            branch.into(),
            "--single-branch".into(),
            url.into(),
            self.workspace.to_string_lossy().into(),
        ];
        // clone 不在 workspace 内执行 (workspace 可能尚未存在)
        let output = Command::new("git")
            .args(&args)
            .output()
            .map_err(|_e| DevnpcError::GitCommand {
                cmd: format!("git {}", args.join(" ")),
                code: -1,
            })?;
        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            return Err(DevnpcError::GitCommand {
                cmd: format!("git {}", args.join(" ")),
                code,
            });
        }
        Ok(())
    }

    /// 创建并切换分支
    pub async fn checkout_branch(&self, branch: &str) -> Result<()> {
        self.run_git_cmd(&["checkout".into(), "-b".into(), branch.into()])
            .map(|_| ())
    }

    /// 提交所有变更 (git add -A + git commit)
    pub async fn commit(&self, message: &str) -> Result<()> {
        self.run_git_cmd(&["add".into(), "-A".into()])?;
        self.run_git_cmd(&["commit".into(), "-m".into(), message.into()])
            .map(|_| ())
    }

    /// 推送分支到 origin
    pub async fn push(&self, branch: &str) -> Result<()> {
        self.run_git_cmd(&[
            "push".into(),
            "-u".into(),
            "origin".into(),
            branch.into(),
        ])
        .map(|_| ())
    }

    /// 获取最近 N 条提交 (git log --oneline -N)
    pub async fn recent_commits(&self, count: usize) -> Result<Vec<String>> {
        let n = format!("-{count}");
        let out = self.run_git_cmd(&["log".into(), "--oneline".into(), n])?;
        Ok(out.lines().map(|s| s.to_string()).collect())
    }

    /// ls-tree HEAD (顶层,非递归),返回原始 stdout 供 repo_index 解析
    pub fn ls_tree_head(&self) -> Result<String> {
        self.run_git_cmd(&["ls-tree".into(), "HEAD".into()])
    }

    /// ls-tree HEAD <path> (指定子目录,非递归)
    pub fn ls_tree_subdir(&self, subdir: &str) -> Result<String> {
        self.run_git_cmd(&["ls-tree".into(), "HEAD".into(), subdir.into()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn run_git_cmd_returns_git_command_error_on_non_zero_exit() {
        let ops = GitOps::new(".");
        let result = ops.run_git_cmd(&["nonexistent-subcommand".to_string()]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::error::DevnpcError::GitCommand { .. }));
    }

    /// 初始化一个临时 git 仓库,写入若干文件并提交,返回 (TempDir, GitOps)
    /// TempDir 必须保持存活,否则目录被删除
    fn setup_temp_repo() -> (TempDir, GitOps) {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        fs::create_dir_all(&repo_path).unwrap();

        // git init
        Command::new("git")
            .args(["init"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        // 配置 user (CI 环境可能无全局配置)
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        // 写文件
        fs::write(repo_path.join("README.md"), "# Test Repo\n").unwrap();
        fs::write(repo_path.join("Cargo.toml"), "[package]\nname=\"t\"\n").unwrap();
        fs::create_dir_all(repo_path.join("src")).unwrap();
        fs::write(repo_path.join("src/main.rs"), "fn main() {}\n").unwrap();

        // git add + commit
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial commit"])
            .current_dir(&repo_path)
            .output()
            .unwrap();

        let ops = GitOps::new(&repo_path);
        (dir, ops)
    }

    #[tokio::test]
    async fn recent_commits_returns_at_least_one_commit() {
        let (_dir, ops) = setup_temp_repo();
        let commits = ops.recent_commits(10).await.unwrap();
        assert!(!commits.is_empty());
        assert!(commits[0].contains("initial commit"));
    }

    #[tokio::test]
    async fn ls_tree_head_returns_top_level_entries() {
        let (_dir, ops) = setup_temp_repo();
        let tree = ops.ls_tree_head().unwrap();
        // 顶层应包含 README.md, Cargo.toml, src
        assert!(tree.contains("README.md"));
        assert!(tree.contains("Cargo.toml"));
        assert!(tree.contains("src"));
    }

    #[tokio::test]
    async fn checkout_branch_creates_new_branch() {
        let (_dir, ops) = setup_temp_repo();
        ops.checkout_branch("npc/test-branch").await.unwrap();
        // 验证当前分支
        let branch = ops
            .run_git_cmd(&["rev-parse".into(), "--abbrev-ref".into(), "HEAD".into()])
            .unwrap();
        assert_eq!(branch, "npc/test-branch");
    }

    #[tokio::test]
    async fn commit_records_new_files() {
        let (_dir, ops) = setup_temp_repo();
        // 写新文件
        fs::write(ops.workspace.join("new_file.txt"), "new content").unwrap();
        ops.commit("add new file").await.unwrap();
        let commits = ops.recent_commits(1).await.unwrap();
        assert!(commits[0].contains("add new file"));
    }
}
