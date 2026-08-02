//! 上下文聚合器
//!
//! 并行获取仓库结构、Issue、PR、CI 历史,聚合为 Context。

use serde::Serialize;

use crate::ci::log_parser::{parse_log, FailureType};
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

/// 从 failed pipelines 提取失败记录,对每个 pipeline 拉取 failed job 日志并解析根因。
///
/// 流程:
/// 1. 过滤 status == "failed" 的 pipeline,取前 N 条 (N = config.max_ci_history_failures)
/// 2. 对每个 failed pipeline: 调用 `get_pipeline_jobs` + `get_job_log` 拉取 failed job 日志
/// 3. 调用 `ci::log_parser::parse_log` 解析日志,映射为 `CiFailure`
/// 4. 若某 job 日志解析无结果,生成 fallback 记录 (job_name 真实, type=Other, cause="job failed")
///
/// 单个 pipeline/job 的日志获取或解析失败不中断整体流程,跳过该条继续。
pub async fn extract_failures_with_logs(
    gitlab: &dyn crate::gitlab_api::GitlabApi,
    project_id: u64,
    pipelines: &[Pipeline],
    config: &ContextConfig,
) -> Vec<CiFailure> {
    let mut result = Vec::new();
    for p in pipelines
        .iter()
        .filter(|p| p.status == "failed")
        .take(config.max_ci_history_failures)
    {
        // 拉取该 pipeline 的 failed job + 日志
        let job_logs = match crate::gitlab_api::pipelines::fetch_failed_job_logs(
            gitlab,
            project_id,
            p.id,
        )
        .await
        {
            Ok(jl) => jl,
            Err(_) => continue, // 日志拉取失败,跳过此 pipeline
        };

        if job_logs.is_empty() {
            // pipeline failed 但无 failed job (可能整体 canceled),生成 fallback
            result.push(CiFailure {
                pipeline_id: p.id,
                job_name: "unknown".to_string(),
                failure_type: FailureType::Other,
                root_cause: "pipeline failed (no failed jobs)".to_string(),
            });
            continue;
        }

        for jl in job_logs {
            let parsed = parse_log(&jl.job.name, &jl.log);
            if parsed.is_empty() {
                // 日志无可识别模式,生成 fallback
                result.push(CiFailure {
                    pipeline_id: p.id,
                    job_name: jl.job.name.clone(),
                    failure_type: FailureType::Other,
                    root_cause: format!("job {} failed (unrecognized log pattern)", jl.job.name),
                });
            } else {
                for pf in parsed {
                    result.push(CiFailure {
                        pipeline_id: p.id,
                        job_name: pf.job_name.clone(),
                        failure_type: pf.failure_type,
                        root_cause: pf.error_message,
                    });
                }
            }
        }
    }
    result
}

impl Context {
    /// 构建上下文
    ///
    /// 并行拉取 Git 仓库结构 + GitLab Issue/PR/Notes/CI 历史,
    /// 随后对 failed pipeline 拉取 job 日志并解析根因。
    pub async fn build(
        gitlab: &dyn crate::gitlab_api::GitlabApi,
        git: &GitOps,
        project_id: u64,
        issue_iid: u64,
        summary_config: &crate::config::SummaryConfig,
        context_config: &ContextConfig,
        project_config: &ProjectConfig,
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
        // 对 failed pipeline 拉取 job 日志并解析根因 (串行,避免与 try_join 的 gitlab 借用冲突)
        let ci_failures = extract_failures_with_logs(gitlab, project_id, &pipelines, context_config).await;

        Ok(Self {
            repo_tree,
            key_files,
            issue,
            related_prs,
            issue_notes,
            recent_commits,
            ci_failures,
            project_config: project_config.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::ops::GitOps;
    use crate::gitlab_api::{CreateMrReq, GitlabApi, Job, NoteAuthor};
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

    fn make_job(id: u64, name: &str, status: &str) -> Job {
        Job {
            id,
            name: name.into(),
            status: status.into(),
            stage: "test".into(),
            web_url: Some(format!("https://gl.test/jobs/{id}")),
        }
    }

    #[tokio::test]
    async fn extract_failures_with_logs_parses_compile_error() {
        let mut jobs = std::collections::HashMap::new();
        jobs.insert(
            100,
            vec![make_job(1, "build", "failed")],
        );
        let mut job_logs = std::collections::HashMap::new();
        job_logs.insert(
            1,
            "error[E0277]: cannot find value `x` in this scope\n  --> src/main.rs:10:5\n".into(),
        );
        let mock = MockGitlab {
            issue: Issue {
                iid: 0,
                title: String::new(),
                description: None,
                state: "opened".into(),
                web_url: String::new(),
            },
            related_mrs: vec![],
            issue_notes: vec![],
            pipelines: vec![make_pipeline(100, "failed")],
            jobs,
            job_logs,
        };
        let failures =
            extract_failures_with_logs(&mock, 1, &mock.pipelines, &default_context_config()).await;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].pipeline_id, 100);
        assert_eq!(failures[0].job_name, "build");
        assert_eq!(failures[0].failure_type, FailureType::Compile);
        assert!(failures[0].root_cause.contains("cannot find value"));
    }

    #[tokio::test]
    async fn extract_failures_with_logs_fallback_on_unrecognized_log() {
        let mut jobs = std::collections::HashMap::new();
        jobs.insert(100, vec![make_job(2, "test", "failed")]);
        let mut job_logs = std::collections::HashMap::new();
        job_logs.insert(2, "some random output\nno pattern here".into());
        let mock = MockGitlab {
            issue: Issue {
                iid: 0,
                title: String::new(),
                description: None,
                state: "opened".into(),
                web_url: String::new(),
            },
            related_mrs: vec![],
            issue_notes: vec![],
            pipelines: vec![make_pipeline(100, "failed")],
            jobs,
            job_logs,
        };
        let failures =
            extract_failures_with_logs(&mock, 1, &mock.pipelines, &default_context_config()).await;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].job_name, "test");
        assert_eq!(failures[0].failure_type, FailureType::Other);
        assert!(failures[0].root_cause.contains("unrecognized log pattern"));
    }

    #[tokio::test]
    async fn extract_failures_with_logs_fallback_when_no_failed_jobs() {
        // pipeline failed 但 jobs 为空 → fallback
        let mock = MockGitlab {
            issue: Issue {
                iid: 0,
                title: String::new(),
                description: None,
                state: "opened".into(),
                web_url: String::new(),
            },
            related_mrs: vec![],
            issue_notes: vec![],
            pipelines: vec![make_pipeline(100, "failed")],
            jobs: std::collections::HashMap::new(),
            job_logs: std::collections::HashMap::new(),
        };
        let failures =
            extract_failures_with_logs(&mock, 1, &mock.pipelines, &default_context_config()).await;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].job_name, "unknown");
        assert_eq!(failures[0].failure_type, FailureType::Other);
        assert!(failures[0].root_cause.contains("no failed jobs"));
    }

    #[tokio::test]
    async fn extract_failures_with_logs_empty_when_no_failed_pipelines() {
        let mock = MockGitlab {
            issue: Issue {
                iid: 0,
                title: String::new(),
                description: None,
                state: "opened".into(),
                web_url: String::new(),
            },
            related_mrs: vec![],
            issue_notes: vec![],
            pipelines: vec![make_pipeline(1, "success")],
            jobs: std::collections::HashMap::new(),
            job_logs: std::collections::HashMap::new(),
        };
        let failures =
            extract_failures_with_logs(&mock, 1, &mock.pipelines, &default_context_config()).await;
        assert!(failures.is_empty());
    }

    #[tokio::test]
    async fn extract_failures_with_logs_caps_at_configured_limit() {
        let mut jobs = std::collections::HashMap::new();
        let mut job_logs = std::collections::HashMap::new();
        for i in 1..=10 {
            jobs.insert(i, vec![make_job(i, "build", "failed")]);
            job_logs.insert(i, format!("error[E0277]: err {i}\n  --> src/main.rs:{i}:1\n"));
        }
        let mock = MockGitlab {
            issue: Issue {
                iid: 0,
                title: String::new(),
                description: None,
                state: "opened".into(),
                web_url: String::new(),
            },
            related_mrs: vec![],
            issue_notes: vec![],
            pipelines: (1..=10).map(|i| make_pipeline(i, "failed")).collect(),
            jobs,
            job_logs,
        };
        let failures =
            extract_failures_with_logs(&mock, 1, &mock.pipelines, &default_context_config()).await;
        // max_ci_history_failures 默认 5
        assert_eq!(failures.len(), 5);
    }

    /// 手写 MockGitlab (避免 mockall async trait 复杂性)
    struct MockGitlab {
        issue: Issue,
        related_mrs: Vec<MergeRequest>,
        issue_notes: Vec<Note>,
        pipelines: Vec<Pipeline>,
        /// 注入的 pipeline jobs (key = pipeline_id)
        jobs: std::collections::HashMap<u64, Vec<Job>>,
        /// 注入的 job 日志 (key = job_id)
        job_logs: std::collections::HashMap<u64, String>,
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
            Ok(MergeRequest {
                iid: 100,
                title: _req.title,
                description: Some(_req.description),
                state: "opened".into(),
                source_branch: _req.source_branch,
                target_branch: _req.target_branch,
                web_url: "https://gl.test/mrs/100".into(),
                draft: _req.draft,
            })
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
            Ok(Note {
                id: 1,
                body: _body.into(),
                author: NoteAuthor {
                    id: 99,
                    username: "devnpc".into(),
                    name: "devnpc bot".into(),
                },
                created_at: "2026-08-01T00:00:00Z".into(),
            })
        }
        async fn create_issue_note(
            &self,
            _project_id: u64,
            _issue_iid: u64,
            _body: &str,
        ) -> Result<Note> {
            Ok(Note {
                id: 1,
                body: _body.into(),
                author: NoteAuthor {
                    id: 99,
                    username: "devnpc".into(),
                    name: "devnpc bot".into(),
                },
                created_at: "2026-08-01T00:00:00Z".into(),
            })
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
            Ok(MergeRequest {
                iid: _mr_iid,
                title: _title.to_string(),
                description: None,
                state: "opened".into(),
                source_branch: "feat/test".into(),
                target_branch: "main".into(),
                web_url: "https://gl.test/mrs/1".into(),
                draft: _draft,
            })
        }
        async fn get_pipeline_jobs(&self, _project_id: u64, pipeline_id: u64) -> Result<Vec<crate::gitlab_api::Job>> {
            Ok(self.jobs.get(&pipeline_id).cloned().unwrap_or_default())
        }
        async fn get_job_log(&self, _project_id: u64, job_id: u64) -> Result<String> {
            Ok(self.job_logs.get(&job_id).cloned().unwrap_or_default())
        }
        async fn get_pipeline(&self, _project_id: u64, _pipeline_id: u64) -> Result<Pipeline> {
            Ok(Pipeline {
                id: 1,
                status: "success".into(),
                ref_: Some("main".into()),
                sha: Some("abc".into()),
                web_url: "https://gl.test/p/1".into(),
            })
        }
        async fn get_file(&self, _project_id: u64, _file_path: &str, _ref_: &str) -> Result<String> {
            Ok(String::new())
        }
        async fn list_tree(
            &self,
            _project_id: u64,
            _path: &str,
            _ref_: &str,
        ) -> Result<Vec<crate::gitlab_api::RepoTreeEntry>> {
            Ok(vec![])
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
            jobs: std::collections::HashMap::new(),
            job_logs: std::collections::HashMap::new(),
        };

        let ctx = Context::build(&mock_gitlab, &ops, 1, 42, &crate::config::SummaryConfig::default(), &default_context_config(), &ProjectConfig::default()).await.unwrap();

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
        // CI 失败 (pipeline 101 failed,无注入 job → fallback "no failed jobs")
        assert_eq!(ctx.ci_failures.len(), 1);
        assert_eq!(ctx.ci_failures[0].pipeline_id, 101);
        assert_eq!(ctx.ci_failures[0].failure_type, FailureType::Other);
        assert!(ctx.ci_failures[0].root_cause.contains("no failed jobs"));
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
