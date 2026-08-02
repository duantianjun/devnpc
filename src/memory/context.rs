//! 上下文聚合器 (P2 完整实现)
//!
//! 并行获取仓库结构、Issue、PR、CI 历史,聚合为 Context。

use serde::Serialize;

use crate::config::{ContextConfig, ProjectConfig};
use crate::error::Result;
use crate::git::ops::GitOps;
use crate::gitlab_api::{Issue, MergeRequest, Note, Pipeline};

/// 聚合的研发记忆
#[derive(Debug, Clone, Serialize)]
pub struct Context {
    pub repo_tree: RepoTree,
    pub key_files: Vec<KeyFile>,
    pub issue: Issue,
    pub related_prs: Vec<MergeRequest>,
    pub issue_notes: Vec<Note>,
    pub recent_commits: Vec<String>,
    pub ci_failures: Vec<CiFailure>,
    pub project_config: ProjectConfig,
}

/// 仓库目录树 (精简)
#[derive(Debug, Clone, Default, Serialize)]
pub struct RepoTree {
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TreeEntry {
    pub path: String,
    pub kind: TreeKind,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum TreeKind {
    File,
    Dir,
}

/// 关键文件摘要
#[derive(Debug, Clone, Serialize)]
pub struct KeyFile {
    pub path: String,
    pub summary: String,
}

/// CI 失败记录
#[derive(Debug, Clone, Serialize)]
pub struct CiFailure {
    pub pipeline_id: u64,
    pub job_name: String,
    pub failure_type: FailureType,
    pub root_cause: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FailureType {
    Compile,
    Test,
    Lint,
    Other,
}

/// 从 pipelines 提取失败记录 (P2 简化版: 仅按 status,无日志解析)
///
/// 详细 job 日志解析留 P4 (ci/log_parser)。
pub fn extract_failures(pipelines: &[Pipeline], config: &ContextConfig) -> Vec<CiFailure> {
    pipelines
        .iter()
        .filter(|p| p.status == "failed")
        .take(config.max_ci_history_failures)
        .map(|p| CiFailure {
            pipeline_id: p.id,
            job_name: "unknown".to_string(),
            failure_type: FailureType::Other,
            root_cause: "pipeline failed".to_string(),
        })
        .collect()
}

impl Context {
    /// 构建上下文 (P2 完整实现)
    ///
    /// 并行拉取 Git 仓库结构 + GitLab Issue/PR/Notes/CI 历史。
    pub async fn build(
        gitlab: &dyn crate::gitlab_api::GitlabApi,
        git: &GitOps,
        project_id: u64,
        issue_iid: u64,
        summary_config: &crate::config::SummaryConfig,
        context_config: &ContextConfig,
    ) -> Result<Self> {
        // 并行: Git 侧 (repo_tree) + GitLab 侧 (issue/related_mrs/notes/pipelines)
        // Git 侧是同步 I/O,用 spawn_blocking 避免阻塞异步运行时
        let workspace_for_tree = git.workspace.clone();
        let (repo_tree, issue, related_prs, issue_notes, recent_commits, pipelines) =
            tokio::try_join!(
                async {
                    tokio::task::spawn_blocking(move || {
                        crate::memory::repo_index::build_repo_tree(&workspace_for_tree)
                    })
                    .await
                    .map_err(|e| {
                        crate::error::DevnpcError::Config(format!("spawn_blocking join error: {e}"))
                    })?
                },
                gitlab.get_issue(project_id, issue_iid),
                gitlab.get_related_mrs(project_id, issue_iid),
                gitlab.get_issue_notes(project_id, issue_iid),
                git.recent_commits(context_config.max_recent_commits),
                gitlab.get_recent_pipelines(project_id, context_config.max_recent_pipelines),
            )?;

        let key_files = crate::memory::repo_index::select_key_files(&repo_tree, &git.workspace, summary_config);
        let ci_failures = extract_failures(&pipelines, context_config);

        // project_config: P2 阶段用默认;完整集成(读 .devnpc.md)留 P3 npc runner
        let project_config = ProjectConfig::default();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::ops::GitOps;
    use crate::gitlab_api::{CreateMrReq, GitlabApi, NoteAuthor};
    use async_trait::async_trait;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn make_pipeline(id: u64, status: &str) -> Pipeline {
        Pipeline {
            id,
            status: status.into(),
            ref_: Some("main".into()),
            sha: Some("abc".into()),
            web_url: format!("https://gl.test/p/{id}"),
        }
    }

    fn default_context_config() -> ContextConfig {
        ContextConfig::default()
    }

    #[test]
    fn extract_failures_filters_failed_pipelines() {
        let pipelines = vec![
            make_pipeline(1, "success"),
            make_pipeline(2, "failed"),
            make_pipeline(3, "running"),
            make_pipeline(4, "failed"),
        ];
        let failures = extract_failures(&pipelines, &default_context_config());
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0].pipeline_id, 2);
        assert_eq!(failures[1].pipeline_id, 4);
    }

    #[test]
    fn extract_failures_caps_at_5() {
        let pipelines: Vec<Pipeline> = (1..=10).map(|i| make_pipeline(i, "failed")).collect();
        let failures = extract_failures(&pipelines, &default_context_config());
        assert_eq!(failures.len(), 5);
    }

    #[test]
    fn extract_failures_sets_other_type_and_default_cause() {
        let pipelines = vec![make_pipeline(1, "failed")];
        let failures = extract_failures(&pipelines, &default_context_config());
        assert_eq!(failures[0].failure_type, FailureType::Other);
        assert_eq!(failures[0].root_cause, "pipeline failed");
        assert_eq!(failures[0].job_name, "unknown");
    }

    #[test]
    fn extract_failures_empty_when_no_failures() {
        let pipelines = vec![make_pipeline(1, "success")];
        let failures = extract_failures(&pipelines, &default_context_config());
        assert!(failures.is_empty());
    }

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
        async fn create_mr_note(
            &self,
            _project_id: u64,
            _mr_iid: u64,
            _body: &str,
        ) -> Result<Note> {
            unimplemented!("mock")
        }
        async fn create_issue_note(
            &self,
            _project_id: u64,
            _issue_iid: u64,
            _body: &str,
        ) -> Result<Note> {
            unimplemented!("mock")
        }
        async fn get_related_mrs(
            &self,
            _project_id: u64,
            _issue_iid: u64,
        ) -> Result<Vec<MergeRequest>> {
            Ok(self.related_mrs.clone())
        }
        async fn get_recent_pipelines(
            &self,
            _project_id: u64,
            _count: usize,
        ) -> Result<Vec<Pipeline>> {
            Ok(self.pipelines.clone())
        }
        async fn update_mr(&self, _project_id: u64, _mr_iid: u64, _title: &str, _draft: bool) -> Result<MergeRequest> {
            unimplemented!("mock")
        }
        async fn get_pipeline_jobs(&self, _project_id: u64, _pipeline_id: u64) -> Result<Vec<crate::gitlab_api::Job>> {
            unimplemented!("mock")
        }
        async fn get_job_log(&self, _project_id: u64, _job_id: u64) -> Result<String> {
            unimplemented!("mock")
        }
    }

    fn setup_temp_repo_with_commits() -> (TempDir, GitOps) {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        fs::create_dir_all(&repo_path).unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "t@t.com"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "T"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        fs::write(
            repo_path.join("Cargo.toml"),
            "[package]\nname=\"t\"\nversion=\"0.1\"\n[dependencies]\ntokio=\"1\"\n",
        )
        .unwrap();
        fs::write(repo_path.join("README.md"), "# Test\n").unwrap();
        fs::create_dir_all(repo_path.join("src")).unwrap();
        fs::write(repo_path.join("src/main.rs"), "fn main() {}\n").unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
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
                author: NoteAuthor {
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

        let ctx = Context::build(&mock_gitlab, &ops, 1, 42, &crate::config::SummaryConfig::default(), &default_context_config()).await.unwrap();

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
        let tree_paths: Vec<&str> =
            ctx.repo_tree.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(tree_paths.contains(&"Cargo.toml"));
        assert!(tree_paths.contains(&"src"));
        // 关键文件
        let key_paths: Vec<&str> =
            ctx.key_files.iter().map(|f| f.path.as_str()).collect();
        assert!(key_paths.contains(&"Cargo.toml"));
        assert!(key_paths.contains(&"README.md"));
        assert!(key_paths.contains(&"src/main.rs"));
    }
}
