//! GitLab HTTP 客户端 (P1 完整实现)

use async_trait::async_trait;

use crate::error::Result;
use super::{CreateMrReq, GitlabApi, Issue, MergeRequest, Note, Pipeline};

/// reqwest 实现
pub struct GitlabClient {
    #[allow(dead_code)]
    base_url: String,
    #[allow(dead_code)]
    token: String,
    #[allow(dead_code)]
    http: reqwest::Client,
}

impl GitlabClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: token.into(),
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl GitlabApi for GitlabClient {
    async fn get_issue(&self, _project_id: u64, _iid: u64) -> Result<Issue> {
        unimplemented!("P1 将实现")
    }

    async fn get_mr(&self, _project_id: u64, _iid: u64) -> Result<MergeRequest> {
        unimplemented!("P1 将实现")
    }

    async fn create_mr(&self, _project_id: u64, _req: CreateMrReq) -> Result<MergeRequest> {
        unimplemented!("P1 将实现")
    }

    async fn get_pipelines(&self, _project_id: u64) -> Result<Vec<Pipeline>> {
        unimplemented!("P1 将实现")
    }

    async fn get_issue_notes(&self, _project_id: u64, _iid: u64) -> Result<Vec<Note>> {
        unimplemented!("P1 将实现")
    }

    async fn get_mr_notes(&self, _project_id: u64, _iid: u64) -> Result<Vec<Note>> {
        unimplemented!("P1 将实现")
    }

    async fn create_mr_note(&self, _project_id: u64, _mr_iid: u64, _body: &str) -> Result<Note> {
        unimplemented!("P1 将实现")
    }
}
