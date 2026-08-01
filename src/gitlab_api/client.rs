//! GitLab HTTP 客户端 (reqwest 实现 GitlabApi trait)
//!
//! 统一封装 GET/POST,处理状态码与错误。

use async_trait::async_trait;
use reqwest::StatusCode;

use crate::error::{DevnpcError, Result};

use super::{CreateMrReq, GitlabApi, Issue, MergeRequest, Note, Pipeline};

/// reqwest 实现
pub struct GitlabClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

impl GitlabClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
            http: reqwest::Client::new(),
        }
    }

    /// 发 GET 请求,返回反序列化的 JSON。
    /// 404 返回 GitlabNotFound,其他非 2xx 返回 GitlabApi。
    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self
            .http
            .get(url)
            .header("PRIVATE-TOKEN", &self.token)
            .send()
            .await?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            let body = resp.text().await.unwrap_or_default();
            return Err(DevnpcError::GitlabNotFound { resource: body });
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(DevnpcError::GitlabApi {
                status: status.as_u16(),
                body,
            });
        }
        Ok(resp.json::<T>().await?)
    }

    /// 发 POST 请求,返回反序列化的 JSON。
    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        form: &[(&str, &str)],
    ) -> Result<T> {
        let resp = self
            .http
            .post(url)
            .header("PRIVATE-TOKEN", &self.token)
            .form(form)
            .send()
            .await?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            let body = resp.text().await.unwrap_or_default();
            return Err(DevnpcError::GitlabNotFound { resource: body });
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(DevnpcError::GitlabApi {
                status: status.as_u16(),
                body,
            });
        }
        Ok(resp.json::<T>().await?)
    }

    fn issue_url(&self, project_id: u64, iid: u64) -> String {
        format!(
            "{}/api/v4/projects/{}/issues/{}",
            self.base_url, project_id, iid
        )
    }

    fn mr_url(&self, project_id: u64, iid: u64) -> String {
        format!(
            "{}/api/v4/projects/{}/merge_requests/{}",
            self.base_url, project_id, iid
        )
    }

    fn mr_notes_url(&self, project_id: u64, mr_iid: u64) -> String {
        format!(
            "{}/api/v4/projects/{}/merge_requests/{}/notes",
            self.base_url, project_id, mr_iid
        )
    }

    fn issue_notes_url(&self, project_id: u64, iid: u64) -> String {
        format!(
            "{}/api/v4/projects/{}/issues/{}/notes",
            self.base_url, project_id, iid
        )
    }

    fn pipelines_url(&self, project_id: u64) -> String {
        format!(
            "{}/api/v4/projects/{}/pipelines",
            self.base_url, project_id
        )
    }
}

#[async_trait]
impl GitlabApi for GitlabClient {
    async fn get_issue(&self, project_id: u64, iid: u64) -> Result<Issue> {
        let url = self.issue_url(project_id, iid);
        self.get(&url).await
    }

    async fn get_mr(&self, project_id: u64, iid: u64) -> Result<MergeRequest> {
        let url = self.mr_url(project_id, iid);
        self.get(&url).await
    }

    async fn create_mr(&self, project_id: u64, req: CreateMrReq) -> Result<MergeRequest> {
        let url = format!(
            "{}/api/v4/projects/{}/merge_requests",
            self.base_url, project_id
        );
        // GitLab MR API: title 前缀 "Draft: " 表草稿
        let title = if req.draft {
            format!("Draft: {}", req.title)
        } else {
            req.title.clone()
        };
        self.post(
            &url,
            &[
                ("source_branch", &req.source_branch),
                ("target_branch", &req.target_branch),
                ("title", &title),
                ("description", &req.description),
            ],
        )
        .await
    }

    async fn get_pipelines(&self, project_id: u64) -> Result<Vec<Pipeline>> {
        let url = self.pipelines_url(project_id);
        self.get(&url).await
    }

    async fn get_issue_notes(&self, project_id: u64, iid: u64) -> Result<Vec<Note>> {
        let url = self.issue_notes_url(project_id, iid);
        self.get(&url).await
    }

    async fn get_mr_notes(&self, project_id: u64, mr_iid: u64) -> Result<Vec<Note>> {
        let url = self.mr_notes_url(project_id, mr_iid);
        self.get(&url).await
    }

    async fn create_mr_note(&self, project_id: u64, mr_iid: u64, body: &str) -> Result<Note> {
        let url = self.mr_notes_url(project_id, mr_iid);
        self.post(&url, &[("body", body)]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab_api::GitlabApi;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(server: &MockServer) -> GitlabClient {
        GitlabClient::new(server.uri(), "test-token")
    }

    #[tokio::test]
    async fn get_issue_returns_parsed_issue() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/issues/42"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "iid": 42,
                "title": "登录 bug",
                "description": "无法登录",
                "state": "opened",
                "web_url": "https://gitlab.test.com/issues/42"
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let issue = client.get_issue(1, 42).await.unwrap();
        assert_eq!(issue.iid, 42);
        assert_eq!(issue.title, "登录 bug");
        assert_eq!(issue.description.as_deref(), Some("无法登录"));
        assert_eq!(issue.state, "opened");
    }

    #[tokio::test]
    async fn get_issue_returns_not_found_error_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/issues/999"))
            .respond_with(ResponseTemplate::new(404).set_body_string("404 Not Found"))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let result = client.get_issue(1, 999).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        // 404 映射为 GitlabNotFound (比 GitlabApi 更具体)
        assert!(matches!(
            err,
            crate::error::DevnpcError::GitlabNotFound { .. }
        ));
    }
}
