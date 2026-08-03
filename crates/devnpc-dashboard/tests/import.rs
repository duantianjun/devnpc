//! 导入流程测试: 构造本地 .jsonl → POST /api/events/import → 校验结果

mod common;

use common::{build_full_jsonl, build_running_jsonl, TestServer};
use devnpc_core::report::event_schema::ImportResult;

/// 构造 multipart 上传并调用导入接口,返回响应
async fn upload(
    server: &TestServer,
    content: &str,
    filename: &str,
) -> reqwest::Response {
    let part = reqwest::multipart::Part::text(content.to_string())
        .file_name(filename.to_string())
        .mime_str("application/x-ndjson")
        .unwrap();
    let form = reqwest::multipart::Form::new().part("file", part);
    server
        .client()
        .post(format!("{}/api/events/import", server.base_url))
        .header("X-Devnpc-Token", &server.token)
        .multipart(form)
        .send()
        .await
        .unwrap()
}

/// 成功导入完整 .jsonl: 返回 200,任务可查询
#[tokio::test]
async fn import_jsonl_creates_task() {
    let server = TestServer::start().await;
    let client = server.client();
    let task_id = format!("imp-{}", uuid::Uuid::new_v4());
    let jsonl = build_full_jsonl(&task_id);

    let resp = upload(&server, &jsonl, &format!("{task_id}.jsonl")).await;
    assert!(
        resp.status().is_success(),
        "导入应返回 2xx, 实际: {}",
        resp.status()
    );
    let result: ImportResult = resp.json().await.unwrap();
    assert_eq!(result.task_id, task_id, "返回的 task_id 应匹配");
    assert_eq!(result.events_count, 2, "应导入 2 条 execution 事件");
    assert!(!result.skipped, "首次导入不应跳过");

    // 校验 GET /api/tasks/:id 返回导入数据
    let resp = client
        .get(format!("{}/api/tasks/{}", server.base_url, task_id))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "导入后任务应可查询");
    let task: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(task["task_id"], task_id);
    assert_eq!(task["status"], "success", "导入的 finished 任务状态应为 success");
}

/// 重复导入已 finish 的文件: 第二次返回 409 Conflict
#[tokio::test]
async fn import_duplicate_finished_returns_409() {
    let server = TestServer::start().await;
    let task_id = format!("dup-{}", uuid::Uuid::new_v4());
    let jsonl = build_full_jsonl(&task_id);

    // 第一次导入成功
    let resp = upload(&server, &jsonl, &format!("{task_id}.jsonl")).await;
    assert!(resp.status().is_success(), "首次导入应成功");

    // 第二次导入同一文件 (task 已 finish) → 409
    let resp = upload(&server, &jsonl, &format!("{task_id}.jsonl")).await;
    assert_eq!(
        resp.status(),
        409,
        "重复导入已 finish 任务应返回 409, 实际: {}",
        resp.status()
    );
}

/// 覆盖导入 running 任务: 再次上传返回 200 (先删后写)
#[tokio::test]
async fn import_overwrites_running_task() {
    let server = TestServer::start().await;
    let client = server.client();
    let task_id = format!("owr-{}", uuid::Uuid::new_v4());
    let running = build_running_jsonl(&task_id); // 仅 task_started + execution,无 finish

    // 第一次导入 running 任务
    let resp = upload(&server, &running, &format!("{task_id}.jsonl")).await;
    assert!(resp.status().is_success(), "首次导入 running 应成功");
    // 校验任务处于 running
    let task: serde_json::Value = client
        .get(format!("{}/api/tasks/{}", server.base_url, task_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(task["status"], "running", "未 finish 的导入任务应为 running");

    // 第二次导入同一 running 文件 → 覆盖,返回 200
    let resp = upload(&server, &running, &format!("{task_id}.jsonl")).await;
    assert_eq!(
        resp.status(),
        200,
        "覆盖 running 任务应返回 200, 实际: {}",
        resp.status()
    );
}

/// 上传格式错误文件: 返回 400 Bad Request
#[tokio::test]
async fn import_malformed_returns_400() {
    let server = TestServer::start().await;
    let bad = "this is not json\n{also not valid json";
    let resp = upload(&server, bad, "bad.jsonl").await;
    assert_eq!(
        resp.status(),
        400,
        "格式错误文件应返回 400, 实际: {}",
        resp.status()
    );
}
