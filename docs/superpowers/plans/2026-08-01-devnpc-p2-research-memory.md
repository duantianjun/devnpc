# devnpc P2 研发记忆 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 实现研发记忆聚合器,并行拉取 Git 仓库结构 + GitLab Issue/PR/Notes/CI 历史,聚合成 `Context` 供 Agent 首轮决策使用。

**Architecture:** `GitOps` 用 `std::process::Command` 调系统 git(clone/checkout/commit/push/ls-tree/log);`memory::repo_index` 构建 RepoTree + 选择关键文件摘要(降 token);`memory::context::Context::build` 用 `tokio::try_join!` 并行聚合 GitLab API + Git 数据。CI 失败提取仅从 pipeline status 推断(详细日志解析留 P4)。

**Tech Stack:** tokio(async), std::process::Command(git), reqwest(GitLab API 复用 P1), wiremock(HTTP mock), tempfile(临时 git 仓库测试)

---

## File Structure

- **Modify:** `src/git/ops.rs` — 实现 git 命令封装(run_git_cmd 辅助 + clone/checkout/commit/push/recent_commits + ls_tree)
- **Modify:** `src/gitlab_api/mod.rs` — 扩展 `GitlabApi` trait(新增 `get_related_mrs` + `get_recent_pipelines`)
- **Modify:** `src/gitlab_api/client.rs` — 实现新 trait 方法 + wiremock 测试
- **Modify:** `src/memory/repo_index.rs` — 实现 `build_repo_tree` + `select_key_files`(含 workspace 参数)
- **Modify:** `src/memory/context.rs` — 实现 `extract_failures` + `Context::build`(并行聚合,加 `project_id` 参数)
- **Test:** 各文件内 `#[cfg(test)] mod tests`(沿用项目约定)

---

### Task 1: git/ops.rs — run_git_cmd 辅助 + clone/checkout/commit/push/recent_commits/ls_tree

**Files:**
- Modify: `src/git/ops.rs`

**目标:** 实现 `GitOps` 全部方法,用 `std::process::Command` 调系统 git。同步执行(CI 单任务,避免 spawn_blocking 复杂度)。

- [x] **Step 1: 写 run_git_cmd 辅助函数测试(命令失败映射 GitCommand 错误)**

在 `src/git/ops.rs` 的 `#[cfg(test)] mod tests` 中追加:

```rust
    use super::*;

    #[test]
    fn run_git_cmd_returns_git_command_error_on_non_zero_exit() {
        let ops = GitOps::new(".");
        // 用一个必然失败的 git 命令 (无效子命令)
        let result = ops.run_git_cmd(&["nonexistent-subcommand".to_string()]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::error::DevnpcError::GitCommand { .. }));
    }
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --lib git::ops::tests::run_git_cmd_returns_git_command_error_on_non_zero_exit`
Expected: FAIL with "method not found `run_git_cmd`" 或编译错误

- [x] **Step 3: 实现 run_git_cmd + 改造 GitOps**

替换 `src/git/ops.rs` 全部内容为:

```rust
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
            .map_err(|e| DevnpcError::GitCommand {
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
            .map_err(|e| DevnpcError::GitCommand {
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

    #[test]
    fn run_git_cmd_returns_git_command_error_on_non_zero_exit() {
        let ops = GitOps::new(".");
        let result = ops.run_git_cmd(&["nonexistent-subcommand".to_string()]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::error::DevnpcError::GitCommand { .. }));
    }
}
```

- [x] **Step 4: 运行测试验证通过**

Run: `cargo test --lib git::ops::tests`
Expected: PASS (1 test)

- [x] **Step 5: 写 recent_commits / ls_tree 集成测试(用临时 git 仓库)**

在 `src/git/ops.rs` 的 `tests` mod 追加:

```rust
    use std::fs;
    use tempfile::TempDir;

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
        fs::write(
            ops.workspace.join("new_file.txt"),
            "new content",
        )
        .unwrap();
        ops.commit("add new file").await.unwrap();
        let commits = ops.recent_commits(1).await.unwrap();
        assert!(commits[0].contains("add new file"));
    }
```

- [x] **Step 6: 运行测试验证通过**

Run: `cargo test --lib git::ops::tests`
Expected: PASS (5 tests)。若环境无 git,测试会失败 — 确认 git 在 PATH 中 (`git --version`)。

- [x] **Step 7: Commit**

```bash
git add src/git/ops.rs
git commit -m "feat: GitOps 实现 (clone/checkout/commit/push/recent_commits/ls_tree)"
```

---

### Task 2: gitlab_api 扩展 — get_related_mrs + get_recent_pipelines

**Files:**
- Modify: `src/gitlab_api/mod.rs`
- Modify: `src/gitlab_api/client.rs`

**目标:** trait 新增 `get_related_mrs`(Issue 的关联 MR)和 `get_recent_pipelines`(限量,避免拉全量)。GitLab API: `GET /projects/:id/issues/:iid/related_merge_requests`。

- [x] **Step 1: 写 get_related_mrs 的 wiremock 测试**

在 `src/gitlab_api/client.rs` 的 `tests` mod 追加:

```rust
    #[tokio::test]
    async fn get_related_mrs_returns_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/issues/42/related_merge_requests"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "iid": 7,
                    "title": "feat: login",
                    "description": "实现登录",
                    "state": "merged",
                    "source_branch": "npc/1-login",
                    "target_branch": "main",
                    "web_url": "https://gl.test/mrs/7",
                    "draft": false
                }
            ])))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let mrs = client.get_related_mrs(1, 42).await.unwrap();
        assert_eq!(mrs.len(), 1);
        assert_eq!(mrs[0].iid, 7);
        assert_eq!(mrs[0].state, "merged");
    }

    #[tokio::test]
    async fn get_recent_pipelines_returns_limited_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines"))
            .and(wiremock::matchers::query_param("per_page", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": 101, "status": "failed", "ref": "main", "sha": "abc", "web_url": "https://gl.test/p/101" }
            ])))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let pipelines = client.get_recent_pipelines(1, 5).await.unwrap();
        assert_eq!(pipelines.len(), 1);
        assert_eq!(pipelines[0].id, 101);
        assert_eq!(pipelines[0].status, "failed");
    }
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --lib gitlab_api::client::tests::get_related_mrs_returns_list`
Expected: FAIL (trait 方法未定义,编译错误)

- [x] **Step 3: 扩展 trait + client 实现**

在 `src/gitlab_api/mod.rs` 的 `GitlabApi` trait 中,于 `create_mr_note` 之后追加两个方法:

```rust
    async fn get_related_mrs(&self, project_id: u64, issue_iid: u64) -> Result<Vec<MergeRequest>>;
    async fn get_recent_pipelines(&self, project_id: u64, count: usize) -> Result<Vec<Pipeline>>;
```

在 `src/gitlab_api/client.rs`:
1. 新增 URL 构造方法(在 `impl GitlabClient` 内,`pipelines_url` 之后):

```rust
    fn related_mrs_url(&self, project_id: u64, issue_iid: u64) -> String {
        format!(
            "{}/api/v4/projects/{}/issues/{}/related_merge_requests",
            self.base_url, project_id, issue_iid
        )
    }

    fn pipelines_url_with_limit(&self, project_id: u64, count: usize) -> String {
        format!(
            "{}/api/v4/projects/{}/pipelines?per_page={}",
            self.base_url, project_id, count
        )
    }
```

2. 在 `impl GitlabApi for GitlabClient` 中,`create_mr_note` 之后追加:

```rust
    async fn get_related_mrs(&self, project_id: u64, issue_iid: u64) -> Result<Vec<MergeRequest>> {
        let url = self.related_mrs_url(project_id, issue_iid);
        self.get(&url).await
    }

    async fn get_recent_pipelines(&self, project_id: u64, count: usize) -> Result<Vec<Pipeline>> {
        let url = self.pipelines_url_with_limit(project_id, count);
        self.get(&url).await
    }
```

- [x] **Step 4: 运行测试验证通过**

Run: `cargo test --lib gitlab_api::client::tests`
Expected: PASS (12 tests: 原 10 + 新 2)

- [x] **Step 5: Commit**

```bash
git add src/gitlab_api/mod.rs src/gitlab_api/client.rs
git commit -m "feat: GitlabApi 扩展 get_related_mrs + get_recent_pipelines"
```

---

### Task 3: memory/repo_index.rs — build_repo_tree

**Files:**
- Modify: `src/memory/repo_index.rs`

**目标:** 用 `git ls-tree HEAD` 解析顶层 + 二级目录,构建 `RepoTree`。解析格式: `<mode> <type> <hash>\t<path>`。

- [x] **Step 1: 写 build_repo_tree 测试(用临时 git 仓库)**

在 `src/memory/repo_index.rs` 的 `tests` mod 追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::ops::GitOps;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn setup_temp_repo() -> (TempDir, GitOps) {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        fs::create_dir_all(&repo_path).unwrap();
        Command::new("git").args(["init"]).current_dir(&repo_path).output().unwrap();
        Command::new("git").args(["config", "user.email", "t@t.com"]).current_dir(&repo_path).output().unwrap();
        Command::new("git").args(["config", "user.name", "T"]).current_dir(&repo_path).output().unwrap();
        fs::write(repo_path.join("Cargo.toml"), "[package]\nname=\"t\"\n").unwrap();
        fs::write(repo_path.join("README.md"), "# T\n").unwrap();
        fs::create_dir_all(repo_path.join("src")).unwrap();
        fs::write(repo_path.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::create_dir_all(repo_path.join("src/handler")).unwrap();
        fs::write(repo_path.join("src/handler/login.rs"), "pub fn login() {}\n").unwrap();
        Command::new("git").args(["add", "-A"]).current_dir(&repo_path).output().unwrap();
        Command::new("git").args(["commit", "-m", "init"]).current_dir(&repo_path).output().unwrap();
        let ops = GitOps::new(&repo_path);
        (dir, ops)
    }

    #[test]
    fn build_repo_tree_contains_top_level_files_and_dirs() {
        let (_dir, ops) = setup_temp_repo();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let paths: Vec<&str> = tree.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"README.md"));
        assert!(paths.contains(&"src"));
        // src 是目录
        let src_entry = tree.entries.iter().find(|e| e.path == "src").unwrap();
        assert_eq!(src_entry.kind, crate::memory::context::TreeKind::Dir);
    }

    #[test]
    fn build_repo_tree_includes_second_level_entries() {
        let (_dir, ops) = setup_temp_repo();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        // 二级: src/main.rs, src/handler
        let paths: Vec<&str> = tree.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&"src/handler"));
    }

    #[test]
    fn build_repo_tree_marks_files_as_file_kind() {
        let (_dir, ops) = setup_temp_repo();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let cargo = tree.entries.iter().find(|e| e.path == "Cargo.toml").unwrap();
        assert_eq!(cargo.kind, crate::memory::context::TreeKind::File);
    }
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --lib memory::repo_index::tests`
Expected: FAIL (build_repo_tree 仍为 unimplemented)

- [x] **Step 3: 实现 build_repo_tree**

替换 `src/memory/repo_index.rs` 全部内容为:

```rust
//! 仓库索引 (P2 实现: 目录树构建、关键文件选择、摘要生成)

use std::path::Path;

use crate::error::Result;
use crate::git::ops::GitOps;
use crate::memory::context::{KeyFile, RepoTree, TreeEntry, TreeKind};

/// 构建仓库目录树 (顶层 + 二级)
///
/// 用 `git ls-tree HEAD` 获取顶层,对顶层目录再 ls-tree 获取二级。
/// 两层结构,token 可控,够 Agent 理解项目布局。
pub fn build_repo_tree(workspace: &Path) -> Result<RepoTree> {
    let ops = GitOps::new(workspace);
    let mut entries = Vec::new();

    // 顶层
    let top_out = ops.ls_tree_head()?;
    for entry in parse_ls_tree(&top_out) {
        entries.push(entry.clone());
        // 对顶层目录,深入二级
        if entry.kind == TreeKind::Dir {
            let sub_out = ops.ls_tree_subdir(&entry.path)?;
            for sub in parse_ls_tree(&sub_out) {
                // sub 的 path 是相对 subdir 的,需要拼接
                let full_path = format!("{}/{}", entry.path, sub.path);
                entries.push(TreeEntry {
                    path: full_path,
                    kind: sub.kind,
                    size: None,
                });
            }
        }
    }

    Ok(RepoTree { entries })
}

/// 解析 `git ls-tree` 输出
/// 行格式: `<mode> <type> <hash>\t<path>`
fn parse_ls_tree(output: &str) -> Vec<TreeEntry> {
    output
        .lines()
        .filter_map(|line| {
            // 找到 tab 分隔 path
            let tab_idx = line.find('\t')?;
            let meta = &line[..tab_idx];
            let path = &line[tab_idx + 1..];
            let parts: Vec<&str> = meta.split_whitespace().collect();
            if parts.len() < 2 {
                return None;
            }
            let kind = match parts[1] {
                "tree" => TreeKind::Dir,
                "blob" => TreeKind::File,
                _ => return None,
            };
            Some(TreeEntry {
                path: path.to_string(),
                kind,
                size: None,
            })
        })
        .collect()
}

/// 选择关键文件 (6 类) 并生成摘要
pub fn select_key_files(tree: &RepoTree, workspace: &Path) -> Vec<KeyFile> {
    // Task 4 实现
    Vec::new()
}
```

- [x] **Step 4: 运行测试验证通过**

Run: `cargo test --lib memory::repo_index::tests`
Expected: PASS (3 tests)

- [x] **Step 5: Commit**

```bash
git add src/memory/repo_index.rs
git commit -m "feat: build_repo_tree 解析 git ls-tree (顶层+二级)"
```

---

### Task 4: memory/repo_index.rs — select_key_files

**Files:**
- Modify: `src/memory/repo_index.rs`

**目标:** 按 6 类规则匹配关键文件,生成摘要(Cargo.toml 取 [dependencies] 段;README.md 前 30 行;src/main.rs 前 50 行;其他源文件不读)。签名加 `workspace: &Path` 用于读文件。

- [x] **Step 1: 写 select_key_files 测试**

在 `src/memory/repo_index.rs` 的 `tests` mod 追加:

```rust
    #[test]
    fn select_key_files_picks_cargo_toml_and_readme() {
        let (_dir, ops) = setup_temp_repo();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let key_files = select_key_files(&tree, &ops.workspace);
        let paths: Vec<&str> = key_files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"README.md"));
    }

    #[test]
    fn select_key_files_cargo_toml_summary_includes_package_name() {
        let (_dir, ops) = setup_temp_repo();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let key_files = select_key_files(&tree, &ops.workspace);
        let cargo = key_files.iter().find(|f| f.path == "Cargo.toml").unwrap();
        // 摘要应包含 [package] 段 (无 [dependencies] 也至少有 package)
        assert!(cargo.summary.contains("[package]") || cargo.summary.contains("name"));
    }

    #[test]
    fn select_key_files_readme_summary_capped_at_30_lines() {
        let (_dir, ops) = setup_temp_repo();
        // 写一个长 README
        let long_readme = (1..=50).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        std::fs::write(ops.workspace.join("README.md"), long_readme).unwrap();
        // 重新提交 (select_key_files 读工作区文件,不需重新 commit)
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let key_files = select_key_files(&tree, &ops.workspace);
        let readme = key_files.iter().find(|f| f.path == "README.md").unwrap();
        let line_count = readme.summary.lines().count();
        assert!(line_count <= 30, "README 摘要应 ≤30 行,实际 {line_count}");
    }

    #[test]
    fn select_key_files_picks_src_main_rs() {
        let (_dir, ops) = setup_temp_repo();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let key_files = select_key_files(&tree, &ops.workspace);
        let paths: Vec<&str> = key_files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"));
    }

    #[test]
    fn select_key_files_ignores_non_key_files() {
        let (_dir, ops) = setup_temp_repo();
        // 加一个非关键文件
        std::fs::write(ops.workspace.join("random.txt"), "data").unwrap();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let key_files = select_key_files(&tree, &ops.workspace);
        let paths: Vec<&str> = key_files.iter().map(|f| f.path.as_str()).collect();
        assert!(!paths.contains(&"random.txt"));
    }
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --lib memory::repo_index::tests`
Expected: FAIL (select_key_files 返回空 Vec,断言失败)

- [x] **Step 3: 实现 select_key_files**

替换 `src/memory/repo_index.rs` 中的 `select_key_files` 函数为:

```rust
/// 关键文件路径匹配规则 (6 类)
const KEY_FILE_PATTERNS: &[&str] = &[
    // 包管理
    "Cargo.toml",
    "package.json",
    "go.mod",
    "pyproject.toml",
    // 文档
    "README.md",
    ".devnpc.md",
    // CI 配置
    ".gitlab-ci.yml",
    // 入口文件
    "src/main.rs",
    "src/lib.rs",
    // 构建脚本
    "Makefile",
    "justfile",
];

/// 选择关键文件 (6 类) 并生成摘要
///
/// 摘要规则 (降 token):
/// - Cargo.toml → 保留 [dependencies] 段 (若存在),否则全文
/// - README.md → 前 30 行
/// - src/main.rs / src/lib.rs → 前 50 行
/// - 其他 → 前 20 行
pub fn select_key_files(tree: &RepoTree, workspace: &Path) -> Vec<KeyFile> {
    let mut key_files = Vec::new();
    for entry in &tree.entries {
        if entry.kind != TreeKind::File {
            continue;
        }
        if !KEY_FILE_PATTERNS.contains(&entry.path.as_str()) {
            continue;
        }
        let full_path = workspace.join(&entry.path);
        let Ok(content) = std::fs::read_to_string(&full_path) else {
            continue;
        };
        let summary = summarize(&entry.path, &content);
        key_files.push(KeyFile {
            path: entry.path.clone(),
            summary,
        });
    }
    key_files
}

/// 按文件类型生成摘要
fn summarize(path: &str, content: &str) -> String {
    match path {
        "Cargo.toml" => summarize_cargo_toml(content),
        "README.md" | ".devnpc.md" => take_first_n_lines(content, 30),
        "src/main.rs" | "src/lib.rs" => take_first_n_lines(content, 50),
        _ => take_first_n_lines(content, 20),
    }
}

/// Cargo.toml 摘要: 保留 [dependencies] 段;若无则返回前 30 行
fn summarize_cargo_toml(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    let mut in_deps = false;
    for line in &lines {
        if line.starts_with('[') {
            in_deps = line.trim() == "[dependencies]";
        }
        if in_deps {
            result.push(*line);
        }
    }
    if result.is_empty() {
        // 无 [dependencies] 段,返回前 30 行
        take_first_n_lines(content, 30)
    } else {
        result.join("\n")
    }
}

/// 取前 N 行
fn take_first_n_lines(content: &str, n: usize) -> String {
    content.lines().take(n).collect::<Vec<_>>().join("\n")
}
```

- [x] **Step 4: 运行测试验证通过**

Run: `cargo test --lib memory::repo_index::tests`
Expected: PASS (8 tests: 原 3 + 新 5)

- [x] **Step 5: Commit**

```bash
git add src/memory/repo_index.rs
git commit -m "feat: select_key_files 按 6 类规则匹配并生成摘要"
```

---

### Task 5: memory/context.rs — extract_failures + 签名调整

**Files:**
- Modify: `src/memory/context.rs`

**目标:** 实现 `extract_failures`(从 pipeline status=="failed" 提取 CiFailure,取最近 5 条)。调整 `Context::build` 签名加 `project_id`。P2 阶段无日志解析,failure_type 设 Other,root_cause 设 "pipeline failed"。

- [x] **Step 1: 写 extract_failures 测试**

在 `src/memory/context.rs` 的 `tests` mod 追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab_api::Pipeline;

    fn make_pipeline(id: u64, status: &str) -> Pipeline {
        Pipeline {
            id,
            status: status.into(),
            ref_: Some("main".into()),
            sha: Some("abc".into()),
            web_url: format!("https://gl.test/p/{id}"),
        }
    }

    #[test]
    fn extract_failures_filters_failed_pipelines() {
        let pipelines = vec![
            make_pipeline(1, "success"),
            make_pipeline(2, "failed"),
            make_pipeline(3, "running"),
            make_pipeline(4, "failed"),
        ];
        let failures = extract_failures(&pipelines);
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0].pipeline_id, 2);
        assert_eq!(failures[1].pipeline_id, 4);
    }

    #[test]
    fn extract_failures_caps_at_5() {
        let pipelines: Vec<Pipeline> = (1..=10)
            .map(|i| make_pipeline(i, "failed"))
            .collect();
        let failures = extract_failures(&pipelines);
        assert_eq!(failures.len(), 5);
    }

    #[test]
    fn extract_failures_sets_other_type_and_default_cause() {
        let pipelines = vec![make_pipeline(1, "failed")];
        let failures = extract_failures(&pipelines);
        assert_eq!(failures[0].failure_type, FailureType::Other);
        assert_eq!(failures[0].root_cause, "pipeline failed");
        assert_eq!(failures[0].job_name, "unknown");
    }

    #[test]
    fn extract_failures_empty_when_no_failures() {
        let pipelines = vec![make_pipeline(1, "success")];
        let failures = extract_failures(&pipelines);
        assert!(failures.is_empty());
    }
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --lib memory::context::tests`
Expected: FAIL (extract_failures 未定义)

- [x] **Step 3: 实现 extract_failures + 调整 build 签名**

在 `src/memory/context.rs` 中:
1. 删除 `impl Context` 中的 `unimplemented!` build 方法,替换为完整实现 + extract_failures 函数:

```rust
/// 从 pipelines 提取失败记录 (P2 简化版: 仅按 status,无日志解析)
///
/// 详细 job 日志解析留 P4 (ci/log_parser)。
pub fn extract_failures(pipelines: &[Pipeline]) -> Vec<CiFailure> {
    pipelines
        .iter()
        .filter(|p| p.status == "failed")
        .take(5)
        .map(|p| CiFailure {
            pipeline_id: p.id,
            job_name: "unknown".to_string(),
            failure_type: FailureType::Other,
            root_cause: "pipeline failed".to_string(),
        })
        .collect()
}
```

2. 修改 `Context::build` 签名(加 `project_id`),实现并行聚合:

```rust
impl Context {
    /// 构建上下文 (P2 完整实现)
    ///
    /// 并行拉取 Git 仓库结构 + GitLab Issue/PR/Notes/CI 历史。
    pub async fn build(
        gitlab: &dyn crate::gitlab_api::GitlabApi,
        git: &GitOps,
        project_id: u64,
        issue_iid: u64,
    ) -> Result<Self> {
        // 并行: Git 侧 (repo_tree) + GitLab 侧 (issue/related_mrs/notes/pipelines)
        let (repo_tree, issue, related_prs, issue_notes, recent_commits, pipelines) =
            tokio::try_join!(
                // Git 侧: 同步操作,用 blocking task
                async {
                    let workspace = git.workspace.clone();
                    tokio::task::spawn_blocking(move || {
                        crate::memory::repo_index::build_repo_tree(&workspace)
                    })
                    .await
                    .map_err(|e| crate::error::DevnpcError::Config(format!("join error: {e}")))?
                },
                gitlab.get_issue(project_id, issue_iid),
                gitlab.get_related_mrs(project_id, issue_iid),
                gitlab.get_issue_notes(project_id, issue_iid),
                git.recent_commits(20),
                gitlab.get_recent_pipelines(project_id, 5),
            )?;

        let repo_tree = repo_tree;
        let key_files = crate::memory::repo_index::select_key_files(&repo_tree, &git.workspace);
        let ci_failures = extract_failures(&pipelines);

        // project_config: 从 .devnpc.md 读取 (复用 config loader,但这里简化为默认)
        // P2 阶段用默认 ProjectConfig;完整集成留 P3 (npc runner 会调 Config::load)
        let project_config = crate::config::ProjectConfig::default();

        Ok(Self {
            repo_tree,
            key_files,
            issue,
            related_prs,
            issue_notes,
            recent_commits,
            ci_failures,
            project_config,
        })
    }
}
```

3. 确认 `Pipeline` 已 import。在文件顶部 `use crate::gitlab_api::{Issue, MergeRequest, Note};` 改为:

```rust
use crate::gitlab_api::{Issue, MergeRequest, Note, Pipeline};
```

- [x] **Step 4: 运行 extract_failures 测试验证通过**

Run: `cargo test --lib memory::context::tests::extract_failures`
Expected: PASS (4 tests)

- [x] **Step 5: 确认整体编译通过(build 签名变更影响)**

Run: `cargo build --lib`
Expected: 编译成功。若有 `ProjectConfig::default()` 缺失,需确认 `ProjectConfig` 派生 `Default`(检查 `src/config/mod.rs`)。

- [x] **Step 6: Commit**

```bash
git add src/memory/context.rs
git commit -m "feat: extract_failures + Context::build 并行聚合 (P2)"
```

---

### Task 6: memory/context.rs — Context::build 集成测试

**Files:**
- Modify: `src/memory/context.rs`

**目标:** 用手写 `MockGitlab` 实现 `GitlabApi` trait,配合临时 git 仓库,验证 `Context::build` 端到端聚合。

- [x] **Step 1: 确认 ProjectConfig 派生 Default**

Read `src/config/mod.rs`,确认 `ProjectConfig` 有 `#[derive(Default)]`。若无,在 Task 5 Step 5 已暴露编译错误,需补 `Default` 派生。

- [x] **Step 2: 写 Context::build 集成测试**

在 `src/memory/context.rs` 的 `tests` mod 追加:

```rust
    use crate::git::ops::GitOps;
    use crate::gitlab_api::{CreateMrReq, GitlabApi};
    use async_trait::async_trait;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    /// 手写 MockGitlab (避免 mockall async trait 复杂性)
    struct MockGitlab {
        issue: Issue,
        related_mrs: Vec<MergeRequest>,
        issue_notes: Vec<Note>,
        pipelines: Vec<Pipeline>,
    }

    #[async_trait]
    impl GitlabApi for MockGitlab {
        async fn get_issue(&self, _project_id: u64, _iid: u64) -> Result<Issue> {
            Ok(self.issue.clone())
        }
        async fn get_mr(&self, _project_id: u64, _iid: u64) -> Result<MergeRequest> {
            Err(crate::error::DevnpcError::GitlabNotFound {
                resource: "mock".into(),
            })
        }
        async fn create_mr(&self, _project_id: u64, _req: CreateMrReq) -> Result<MergeRequest> {
            unimplemented!("mock")
        }
        async fn get_pipelines(&self, _project_id: u64) -> Result<Vec<Pipeline>> {
            Ok(self.pipelines.clone())
        }
        async fn get_issue_notes(&self, _project_id: u64, _iid: u64) -> Result<Vec<Note>> {
            Ok(self.issue_notes.clone())
        }
        async fn get_mr_notes(&self, _project_id: u64, _mr_iid: u64) -> Result<Vec<Note>> {
            Ok(vec![])
        }
        async fn create_mr_note(&self, _project_id: u64, _mr_iid: u64, _body: &str) -> Result<Note> {
            unimplemented!("mock")
        }
        async fn get_related_mrs(&self, _project_id: u64, _issue_iid: u64) -> Result<Vec<MergeRequest>> {
            Ok(self.related_mrs.clone())
        }
        async fn get_recent_pipelines(&self, _project_id: u64, _count: usize) -> Result<Vec<Pipeline>> {
            Ok(self.pipelines.clone())
        }
    }

    fn setup_temp_repo_with_commits() -> (TempDir, GitOps) {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        fs::create_dir_all(&repo_path).unwrap();
        Command::new("git").args(["init"]).current_dir(&repo_path).output().unwrap();
        Command::new("git").args(["config", "user.email", "t@t.com"]).current_dir(&repo_path).output().unwrap();
        Command::new("git").args(["config", "user.name", "T"]).current_dir(&repo_path).output().unwrap();
        fs::write(repo_path.join("Cargo.toml"), "[package]\nname=\"t\"\nversion=\"0.1\"\n[dependencies]\ntokio=\"1\"\n").unwrap();
        fs::write(repo_path.join("README.md"), "# Test\n").unwrap();
        fs::create_dir_all(repo_path.join("src")).unwrap();
        fs::write(repo_path.join("src/main.rs"), "fn main() {}\n").unwrap();
        Command::new("git").args(["add", "-A"]).current_dir(&repo_path).output().unwrap();
        Command::new("git").args(["commit", "-m", "init"]).current_dir(&repo_path).output().unwrap();
        let ops = GitOps::new(&repo_path);
        (dir, ops)
    }

    #[tokio::test]
    async fn context_build_aggregates_all_sources() {
        let (_dir, ops) = setup_temp_repo_with_commits();

        let mock_gitlab = MockGitlab {
            issue: Issue {
                iid: 42,
                title: "登录 bug".into(),
                description: Some("无法登录".into()),
                state: "opened".into(),
                web_url: "https://gl.test/issues/42".into(),
            },
            related_mrs: vec![MergeRequest {
                iid: 7,
                title: "feat: login".into(),
                description: Some("实现".into()),
                state: "merged".into(),
                source_branch: "npc/1".into(),
                target_branch: "main".into(),
                web_url: "https://gl.test/mrs/7".into(),
                draft: false,
            }],
            issue_notes: vec![Note {
                id: 1,
                body: "@devnpc 修复".into(),
                author: crate::gitlab_api::NoteAuthor {
                    id: 10,
                    username: "alice".into(),
                    name: "Alice".into(),
                },
                created_at: "2026-08-01T10:00:00Z".into(),
            }],
            pipelines: vec![
                make_pipeline(100, "success"),
                make_pipeline(101, "failed"),
            ],
        };

        let ctx = Context::build(&mock_gitlab, &ops, 1, 42).await.unwrap();

        // Issue
        assert_eq!(ctx.issue.iid, 42);
        assert_eq!(ctx.issue.title, "登录 bug");
        // 相关 PR
        assert_eq!(ctx.related_prs.len(), 1);
        assert_eq!(ctx.related_prs[0].iid, 7);
        // Notes
        assert_eq!(ctx.issue_notes.len(), 1);
        assert_eq!(ctx.issue_notes[0].body, "@devnpc 修复");
        // 最近提交
        assert!(!ctx.recent_commits.is_empty());
        // CI 失败
        assert_eq!(ctx.ci_failures.len(), 1);
        assert_eq!(ctx.ci_failures[0].pipeline_id, 101);
        // Repo tree
        let tree_paths: Vec<&str> = ctx.repo_tree.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(tree_paths.contains(&"Cargo.toml"));
        assert!(tree_paths.contains(&"src"));
        // 关键文件
        let key_paths: Vec<&str> = ctx.key_files.iter().map(|f| f.path.as_str()).collect();
        assert!(key_paths.contains(&"Cargo.toml"));
        assert!(key_paths.contains(&"README.md"));
        assert!(key_paths.contains(&"src/main.rs"));
    }
```

- [x] **Step 3: 运行测试验证通过**

Run: `cargo test --lib memory::context::tests`
Expected: PASS (5 tests: 4 extract_failures + 1 build 集成)

- [x] **Step 4: Commit**

```bash
git add src/memory/context.rs
git commit -m "test: Context::build 端到端聚合测试 (MockGitlab + 临时仓库)"
```

---

### Task 7: 全量测试 + clippy + 验收

**Files:**
- 无修改(仅验证)

- [x] **Step 1: 全量测试**

Run: `cargo test --all`
Expected: 所有测试通过(原 44 + P2 新增约 18 = ~62)

- [x] **Step 2: clippy 严格检查**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 无警告

- [x] **Step 3: release 构建**

Run: `cargo build --release`
Expected: 成功

- [x] **Step 4: CLI 冒烟**

```powershell
$env:DEVNPC_API_KEY="sk-test1234567890abcdef"
$env:DEVNPC_BASE_URL="https://api.example.com/v1"
$env:DEVNPC_MODEL="gpt-4o"
$env:GITLAB_URL="https://gitlab.example.com"
$env:GITLAB_TOKEN="glpat-test"
$env:CI_PROJECT_ID="1"
.\target\release\devnpc.exe info
.\target\release\devnpc.exe config
```
Expected: 两条命令均正常输出,退出码 0

- [x] **Step 5: Commit 收尾(若有未提交改动)**

```bash
git status
# 若有改动:
git add -A
git commit -m "chore: P2 收尾"
```

---

## Self-Review 核对

**Spec 覆盖:**
- 设计 5.1 记忆来源: 仓库结构(build_repo_tree)✓、Issue(get_issue)✓、相关 PR(get_related_mrs)✓、Issue 评论(get_issue_notes)✓、CI 历史(get_recent_pipelines)✓、最近提交(recent_commits)✓。项目指令(.devnpc.md)在 P1 config 已实现,P2 build 用默认 ProjectConfig(完整集成留 P3)。
- 设计 5.2 Context::build 并行聚合: tokio::try_join! ✓
- 设计 5.3 仓库结构索引: RepoTree + 6 类关键文件 + 摘要规则 ✓
- 设计 5.4 CI 失败提取: extract_failures(简化版,无日志,留 P4)✓
- 设计 5.5 上下文预算: P2 不实现 token 截断(YAGNI,P3 prompt 构建时按需)

**Placeholder 扫描:** 无 TBD/TODO;每个步骤含完整代码。

**Type 一致性:**
- `build_repo_tree(workspace: &Path) -> Result<RepoTree>` — Task 3 定义,Task 5/6 调用一致
- `select_key_files(tree: &RepoTree, workspace: &Path) -> Vec<KeyFile>` — Task 4 定义(加 workspace),Task 5 调用一致
- `extract_failures(pipelines: &[Pipeline]) -> Vec<CiFailure>` — Task 5 定义/调用一致
- `Context::build(gitlab, git, project_id, issue_iid)` — Task 5 定义,Task 6 测试调用一致
- `GitlabApi::get_related_mrs(project_id, issue_iid)` / `get_recent_pipelines(project_id, count)` — Task 2 定义,Task 5/6 调用一致
- `GitOps::ls_tree_head() / ls_tree_subdir()` — Task 1 定义,Task 3 调用一致

**潜在风险:**
- `ProjectConfig::default()` 需确认派生 Default(Task 5 Step 5 会暴露)
- git 测试依赖系统 git 在 PATH(Task 1 Step 6 会暴露)
- `tokio::task::spawn_blocking` 要求闭包 'static + Send,workspace 用 clone 移入(Task 5 已处理)
