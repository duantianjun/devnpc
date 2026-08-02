//! GitLab REST API v4 客户端

pub mod client;
pub mod issues;
pub mod mrs;
pub mod notes;
pub mod pipelines;
pub mod repo;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::Result;

/// GitLab API 抽象 trait (便于 mock 测试)
#[async_trait]
pub trait GitlabApi: Send + Sync {
    async fn get_issue(&self, project_id: u64, iid: u64) -> Result<Issue>;
    async fn get_mr(&self, project_id: u64, iid: u64) -> Result<MergeRequest>;
    async fn create_mr(&self, project_id: u64, req: CreateMrReq) -> Result<MergeRequest>;
    async fn get_pipelines(&self, project_id: u64) -> Result<Vec<Pipeline>>;
    async fn get_issue_notes(&self, project_id: u64, iid: u64) -> Result<Vec<Note>>;
    async fn get_mr_notes(&self, project_id: u64, mr_iid: u64) -> Result<Vec<Note>>;
    async fn create_mr_note(&self, project_id: u64, mr_iid: u64, body: &str) -> Result<Note>;
    async fn create_issue_note(&self, project_id: u64, issue_iid: u64, body: &str) -> Result<Note>;
    async fn get_related_mrs(&self, project_id: u64, issue_iid: u64) -> Result<Vec<MergeRequest>>;
    async fn get_recent_pipelines(&self, project_id: u64, count: usize) -> Result<Vec<Pipeline>>;

    // === P4 新增: CI 闭环控制 ===

    /// 更新 MR (用于移除 Draft 状态)
    async fn update_mr(&self, project_id: u64, mr_iid: u64, title: &str, draft: bool) -> Result<MergeRequest>;

    /// 获取 pipeline 的 job 列表
    async fn get_pipeline_jobs(&self, project_id: u64, pipeline_id: u64) -> Result<Vec<Job>>;

    /// 获取 job 的原始日志
    async fn get_job_log(&self, project_id: u64, job_id: u64) -> Result<String>;
}

// === 数据模型 ===

#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub iid: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub web_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MergeRequest {
    pub iid: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub source_branch: String,
    pub target_branch: String,
    pub web_url: String,
    pub draft: bool,
}

#[derive(Debug, Clone)]
pub struct CreateMrReq {
    pub source_branch: String,
    pub target_branch: String,
    pub title: String,
    pub description: String,
    pub draft: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    #[serde(rename = "ref")]
    pub ref_: Option<String>,
    pub sha: Option<String>,
    pub web_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub status: String,
    pub stage: String,
    pub web_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Note {
    pub id: u64,
    pub body: String,
    pub author: NoteAuthor,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NoteAuthor {
    pub id: u64,
    pub username: String,
    pub name: String,
}
