//! GitLab API 工具 (P3 实现: create_mr_note)

use std::sync::Arc;

pub struct GitlabTool {
    #[allow(dead_code)]
    pub client: Arc<dyn crate::gitlab_api::GitlabApi>,
    #[allow(dead_code)]
    pub project_id: u64,
}

impl GitlabTool {
    pub fn new(client: Arc<dyn crate::gitlab_api::GitlabApi>, project_id: u64) -> Self {
        Self { client, project_id }
    }
}
