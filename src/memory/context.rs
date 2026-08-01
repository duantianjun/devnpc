//! 上下文聚合器 (P2 完整实现)
//!
//! 并行获取仓库结构、Issue、PR、CI 历史,聚合为 Context。

use crate::config::ProjectConfig;
use crate::error::Result;
use crate::git::ops::GitOps;
use crate::gitlab_api::{Issue, MergeRequest, Note};

/// 聚合的研发记忆
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone, Default)]
pub struct RepoTree {
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub path: String,
    pub kind: TreeKind,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeKind {
    File,
    Dir,
}

/// 关键文件摘要
#[derive(Debug, Clone)]
pub struct KeyFile {
    pub path: String,
    pub summary: String,
}

/// CI 失败记录
#[derive(Debug, Clone)]
pub struct CiFailure {
    pub pipeline_id: u64,
    pub job_name: String,
    pub failure_type: FailureType,
    pub root_cause: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureType {
    Compile,
    Test,
    Lint,
    Other,
}

impl Context {
    /// 构建上下文 (P2 完整实现)
    pub async fn build(
        _gitlab: &dyn crate::gitlab_api::GitlabApi,
        _git: &GitOps,
        _issue_iid: u64,
    ) -> Result<Self> {
        unimplemented!("P2 将实现")
    }
}
