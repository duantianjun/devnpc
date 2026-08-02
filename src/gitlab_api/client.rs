//! GitLab HTTP 客户端 (reqwest 实现 GitlabApi trait)
//!
//! 统一封装 GET/POST,处理状态码与错误。

use async_trait::async_trait;
use reqwest::StatusCode;

use crate::error::{DevnpcError, Result};

use super::{CreateMrReq, GitlabApi, Issue, Job, MergeRequest, Note, Pipeline};

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

    /// 发 PUT 请求,返回反序列化的 JSON。
    async fn put<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        form: &[(&str, &str)],
    ) -> Result<T> {
        let resp = self
            .http
            .put(url)
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

    /// 发 GET 请求,返回原始文本 (用于 job 日志)
    async fn get_raw(&self, url: &str) -> Result<String> {
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
        Ok(resp.text().await?)
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

    fn pipeline_jobs_url(&self, project_id: u64, pipeline_id: u64) -> String {
        format!(
            "{}/api/v4/projects/{}/pipelines/{}/jobs",
            self.base_url, project_id, pipeline_id
        )
    }

    fn job_log_url(&self, project_id: u64, job_id: u64) -> String {
        format!(
            "{}/api/v4/projects/{}/jobs/{}/trace",
            self.base_url, project_id, job_id
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

    async fn create_issue_note(&self, project_id: u64, issue_iid: u64, body: &str) -> Result<Note> {
        let url = self.issue_notes_url(project_id, issue_iid);
        self.post(&url, &[("body", body)]).await
    }

    async fn get_related_mrs(&self, project_id: u64, issue_iid: u64) -> Result<Vec<MergeRequest>> {
        let url = self.related_mrs_url(project_id, issue_iid);
        self.get(&url).await
    }

    async fn get_recent_pipelines(&self, project_id: u64, count: usize) -> Result<Vec<Pipeline>> {
        let url = self.pipelines_url_with_limit(project_id, count);
        self.get(&url).await
    }

    async fn update_mr(&self, project_id: u64, mr_iid: u64, title: &str, draft: bool) -> Result<MergeRequest> {
        let url = self.mr_url(project_id, mr_iid);
        let title_val = if draft {
            format!("Draft: {}", title)
        } else {
            // 移除 "Draft: " 前缀
            title.trim_start_matches("Draft: ").to_string()
        };
        self.put(
            &url,
            &[("title", &title_val)],
        )
        .await
    }

    async fn get_pipeline_jobs(&self, project_id: u64, pipeline_id: u64) -> Result<Vec<Job>> {
        let url = self.pipeline_jobs_url(project_id, pipeline_id);
        self.get(&url).await
    }

    async fn get_job_log(&self, project_id: u64, job_id: u64) -> Result<String> {
        let url = self.job_log_url(project_id, job_id);
        self.get_raw(&url).await
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

    #[tokio::test]
    async fn get_mr_returns_parsed_mr() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/merge_requests/7"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "iid": 7,
                "title": "feat: add login",
                "description": "实现登录",
                "state": "opened",
                "source_branch": "npc/1-login",
                "target_branch": "main",
                "web_url": "https://gitlab.test.com/mrs/7",
                "draft": false,
                "work_in_progress": false
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let mr = client.get_mr(1, 7).await.unwrap();
        assert_eq!(mr.iid, 7);
        assert_eq!(mr.source_branch, "npc/1-login");
        assert_eq!(mr.target_branch, "main");
        assert!(!mr.draft);
    }

    #[tokio::test]
    async fn create_mr_posts_form_and_returns_mr() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v4/projects/1/merge_requests"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .and(wiremock::matchers::body_string_contains(
                "source_branch=npc%2F1-login",
            ))
            .and(wiremock::matchers::body_string_contains("target_branch=main"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "iid": 8,
                "title": "Draft: feat: login",
                "description": "desc",
                "state": "opened",
                "source_branch": "npc/1-login",
                "target_branch": "main",
                "web_url": "https://gitlab.test.com/mrs/8",
                "draft": true,
                "work_in_progress": true
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let req = CreateMrReq {
            source_branch: "npc/1-login".into(),
            target_branch: "main".into(),
            title: "feat: login".into(),
            description: "desc".into(),
            draft: true,
        };
        let mr = client.create_mr(1, req).await.unwrap();
        assert_eq!(mr.iid, 8);
        assert!(mr.draft);
    }

    #[tokio::test]
    async fn get_mr_returns_api_error_on_500() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/merge_requests/1"))
            .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client.get_mr(1, 1).await.unwrap_err();
        assert!(matches!(
            err,
            crate::error::DevnpcError::GitlabApi { status: 500, .. }
        ));
    }

    #[tokio::test]
    async fn get_pipelines_returns_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": 100, "status": "success", "ref": "main", "sha": "abc123", "web_url": "https://gl.test/p/100" },
                { "id": 101, "status": "failed", "ref": "npc/1-x", "sha": "def456", "web_url": "https://gl.test/p/101" }
            ])))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let pipelines = client.get_pipelines(1).await.unwrap();
        assert_eq!(pipelines.len(), 2);
        assert_eq!(pipelines[0].id, 100);
        assert_eq!(pipelines[0].status, "success");
        assert_eq!(pipelines[0].ref_.as_deref(), Some("main"));
        assert_eq!(pipelines[1].status, "failed");
    }

    #[tokio::test]
    async fn get_issue_notes_returns_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/issues/42/notes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": 1,
                    "body": "@devnpc 修复登录",
                    "author": { "id": 10, "username": "alice", "name": "Alice" },
                    "created_at": "2026-08-01T10:00:00Z"
                }
            ])))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let notes = client.get_issue_notes(1, 42).await.unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].body, "@devnpc 修复登录");
        assert_eq!(notes[0].author.username, "alice");
    }

    #[tokio::test]
    async fn get_mr_notes_returns_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/merge_requests/7/notes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": 5,
                    "body": "CI 通过",
                    "author": { "id": 11, "username": "bob", "name": "Bob" },
                    "created_at": "2026-08-01T11:00:00Z"
                }
            ])))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let notes = client.get_mr_notes(1, 7).await.unwrap();
        assert_eq!(notes[0].author.name, "Bob");
    }

    #[tokio::test]
    async fn create_mr_note_posts_body_and_returns_note() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v4/projects/1/merge_requests/7/notes"))
            .and(wiremock::matchers::body_string_contains(
                "body=CI+%E9%80%9A%E8%BF%87",
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": 9,
                "body": "CI 通过",
                "author": { "id": 99, "username": "devnpc", "name": "devnpc bot" },
                "created_at": "2026-08-01T12:00:00Z"
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let note = client.create_mr_note(1, 7, "CI 通过").await.unwrap();
        assert_eq!(note.id, 9);
        assert_eq!(note.author.username, "devnpc");
    }

    #[tokio::test]
    async fn get_pipelines_returns_not_found_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/999/pipelines"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client.get_pipelines(999).await.unwrap_err();
        assert!(matches!(
            err,
            crate::error::DevnpcError::GitlabNotFound { .. }
        ));
    }

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
}
