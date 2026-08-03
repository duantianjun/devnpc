//! GitLab HTTP 客户端 (reqwest 实现 GitlabApi trait)
//!
//! 统一封装 GET/POST,处理状态码与错误。

use async_trait::async_trait;
use reqwest::{Method, StatusCode};

use crate::error::{DevnpcError, Result};

use super::{
    CreateMrReq, GitlabApi, Issue, Job, MergeRequest, MergeRequestChange, Note, Pipeline,
    RepoTreeEntry,
};

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

    /// 校验响应状态码,失败时消费 resp 并提取错误体。
    ///
    /// - 404 → `GitlabNotFound` (携带响应体作为 resource)
    /// - 其他非 2xx → `GitlabApi` (携带 status + body)
    /// - 2xx → 返回 resp 供调用方继续读取 body
    async fn ensure_success(&self, resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            let body = resp.text().await.unwrap_or_default();
            Err(DevnpcError::GitlabNotFound { resource: body })
        } else if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            Err(DevnpcError::GitlabApi {
                status: status.as_u16(),
                body,
            })
        } else {
            Ok(resp)
        }
    }

    /// 通用请求构建 + 发送 + 状态校验。
    ///
    /// 自动注入 `PRIVATE-TOKEN` 头,可选附加 form 表单,发送后统一校验状态码。
    /// 成功时返回已校验的 `reqwest::Response`,由调用方决定按 JSON 还是 text 读取。
    async fn send(
        &self,
        method: Method,
        url: &str,
        form: Option<&[(&str, &str)]>,
    ) -> Result<reqwest::Response> {
        let mut req = self
            .http
            .request(method, url)
            .header("PRIVATE-TOKEN", &self.token);
        if let Some(form) = form {
            req = req.form(form);
        }
        let resp = req.send().await?;
        self.ensure_success(resp).await
    }

    /// 发 GET 请求,返回反序列化的 JSON。
    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self.send(Method::GET, url, None).await?;
        Ok(resp.json::<T>().await?)
    }

    /// 发 PUT 请求,返回反序列化的 JSON。
    async fn put<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        form: &[(&str, &str)],
    ) -> Result<T> {
        let resp = self.send(Method::PUT, url, Some(form)).await?;
        Ok(resp.json::<T>().await?)
    }

    /// 发 GET 请求,返回原始文本 (用于 job 日志)
    async fn get_raw(&self, url: &str) -> Result<String> {
        let resp = self.send(Method::GET, url, None).await?;
        Ok(resp.text().await?)
    }

    /// 发 POST 请求,返回反序列化的 JSON。
    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        form: &[(&str, &str)],
    ) -> Result<T> {
        let resp = self.send(Method::POST, url, Some(form)).await?;
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

    fn mr_changes_url(&self, project_id: u64, mr_iid: u64) -> String {
        format!(
            "{}/api/v4/projects/{}/merge_requests/{}/changes",
            self.base_url, project_id, mr_iid
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

    fn pipeline_url(&self, project_id: u64, pipeline_id: u64) -> String {
        format!(
            "{}/api/v4/projects/{}/pipelines/{}",
            self.base_url, project_id, pipeline_id
        )
    }

    /// 仓库文件 URL: file_path 整体编码 (含 `/` → `%2F`),ref_ 做 URL 编码,使用 `?raw=1` 取原始内容
    fn file_url(&self, project_id: u64, file_path: &str, ref_: &str) -> String {
        format!(
            "{}/api/v4/projects/{}/repository/files/{}?ref={}&raw=1",
            self.base_url, project_id, encode_uri_component(file_path), encode_uri_component(ref_)
        )
    }

    /// 仓库目录树 URL: path 留空时表示根目录,path 和 ref_ 做 URL 编码
    fn tree_url(&self, project_id: u64, path: &str, ref_: &str) -> String {
        let ref_encoded = encode_uri_component(ref_);
        if path.is_empty() {
            format!(
                "{}/api/v4/projects/{}/repository/tree?ref={}",
                self.base_url, project_id, ref_encoded
            )
        } else {
            format!(
                "{}/api/v4/projects/{}/repository/tree?path={}&ref={}",
                self.base_url, project_id, encode_uri_component(path), ref_encoded
            )
        }
    }
}

/// URL 查询参数编码 (RFC 3986 percent-encoding)
///
/// 编码所有非保留字符 (A-Z a-z 0-9 - _ . ~ 之外的字符全部编码为 %XX)。
/// 用于 GitLab API 的 file_path / ref_ / path 参数,防止 `&` `#` `?` 等字符注入额外查询参数。
fn encode_uri_component(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                char::from(b).to_string()
            } else {
                format!("%{:02X}", b)
            }
        })
        .collect()
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

    async fn get_pipeline(&self, project_id: u64, pipeline_id: u64) -> Result<Pipeline> {
        let url = self.pipeline_url(project_id, pipeline_id);
        self.get(&url).await
    }

    async fn get_file(&self, project_id: u64, file_path: &str, ref_: &str) -> Result<String> {
        let url = self.file_url(project_id, file_path, ref_);
        self.get_raw(&url).await
    }

    async fn list_tree(
        &self,
        project_id: u64,
        path: &str,
        ref_: &str,
    ) -> Result<Vec<RepoTreeEntry>> {
        let url = self.tree_url(project_id, path, ref_);
        self.get(&url).await
    }

    async fn get_mr_changes(
        &self,
        project_id: u64,
        mr_iid: u64,
    ) -> Result<Vec<MergeRequestChange>> {
        let url = self.mr_changes_url(project_id, mr_iid);
        // GET /projects/:id/merge_requests/:mr_iid/changes 返回包装对象: {"changes": [...]}
        let resp = self.send(reqwest::Method::GET, &url, None).await?;
        let json: serde_json::Value = resp.json().await?;
        let changes: Vec<MergeRequestChange> = json
            .get("changes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(changes)
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

    #[tokio::test]
    async fn get_pipeline_returns_single_pipeline() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines/100"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 100,
                "status": "running",
                "ref": "main",
                "sha": "abc123",
                "web_url": "https://gl.test/p/100"
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let p = client.get_pipeline(1, 100).await.unwrap();
        assert_eq!(p.id, 100);
        assert_eq!(p.status, "running");
        assert_eq!(p.ref_.as_deref(), Some("main"));
    }

    #[tokio::test]
    async fn get_pipeline_returns_not_found_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines/999"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client.get_pipeline(1, 999).await.unwrap_err();
        assert!(matches!(
            err,
            crate::error::DevnpcError::GitlabNotFound { .. }
        ));
    }

    #[tokio::test]
    async fn get_file_returns_raw_content() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/repository/files/src%2Fmain.rs"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .and(wiremock::matchers::query_param("ref", "main"))
            .and(wiremock::matchers::query_param("raw", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_string("fn main() {}\n"))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let content = client.get_file(1, "src/main.rs", "main").await.unwrap();
        assert_eq!(content, "fn main() {}\n");
    }

    #[tokio::test]
    async fn get_file_returns_not_found_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/repository/files/missing.rs"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client.get_file(1, "missing.rs", "main").await.unwrap_err();
        assert!(matches!(
            err,
            crate::error::DevnpcError::GitlabNotFound { .. }
        ));
    }

    #[tokio::test]
    async fn list_tree_returns_entries() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/repository/tree"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .and(wiremock::matchers::query_param("ref", "main"))
            .and(wiremock::matchers::query_param("path", "src"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": "abc1",
                    "name": "main.rs",
                    "type": "blob",
                    "path": "src/main.rs",
                    "mode": "100644"
                },
                {
                    "id": "abc2",
                    "name": "lib",
                    "type": "tree",
                    "path": "src/lib",
                    "mode": "040000"
                }
            ])))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let entries = client.list_tree(1, "src", "main").await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "main.rs");
        assert_eq!(entries[0].type_, "blob");
        assert_eq!(entries[1].type_, "tree");
        assert_eq!(entries[1].path, "src/lib");
    }

    #[tokio::test]
    async fn list_tree_root_uses_empty_path() {
        let server = MockServer::start().await;
        // 根目录: 无 path 参数
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/repository/tree"))
            .and(wiremock::matchers::query_param("ref", "main"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": "r1",
                    "name": "README.md",
                    "type": "blob",
                    "path": "README.md",
                    "mode": "100644"
                }
            ])))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let entries = client.list_tree(1, "", "main").await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "README.md");
    }

    #[tokio::test]
    async fn get_mr_changes_parses_changes_array() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/merge_requests/7/changes"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "changes": [
                    {
                        "old_path": "src/auth.rs",
                        "new_path": "src/auth.rs",
                        "new_file": false,
                        "renamed_file": false,
                        "deleted_file": false
                    },
                    {
                        "old_path": "src/old.rs",
                        "new_path": "src/new.rs",
                        "new_file": false,
                        "renamed_file": true,
                        "deleted_file": false
                    }
                ]
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let changes = client.get_mr_changes(1, 7).await.unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].new_path, "src/auth.rs");
        assert_eq!(changes[1].old_path, "src/old.rs");
        assert!(changes[1].renamed_file);
    }

    #[tokio::test]
    async fn get_mr_changes_handles_empty_or_missing_changes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/merge_requests/8/changes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "changes": []
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let changes = client.get_mr_changes(1, 8).await.unwrap();
        assert!(changes.is_empty());
    }
}
