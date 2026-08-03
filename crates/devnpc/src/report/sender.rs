//! Dashboard 事件推送器与本地事件记录器 (spec §4.2)
//!
//! 两个独立组件:
//! - `LocalEventLogger`: 本地 .jsonl 文件记录 (兜底机制,独立于推送)
//! - `EventSender`: channel + 异步批量 POST 推送 (仅 dashboard.enabled=true 时创建)

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use devnpc_core::report::event_schema::{
    BatchEventsRequest, ExecutionEvent, EventLogEntry, TaskFinishedEvent, TaskStartedEvent,
};

use crate::config::DashboardConfig;

// ============================================================
// LocalEventLogger
// ============================================================

/// 本地事件记录器 (兜底机制,独立于推送)
///
/// 即使 dashboard 未配置,只要 `local_event_log=true` 就会创建。
/// 文件格式: JSON Lines (.jsonl),每行一个 `EventLogEntry`。
/// 文件位置: 与 HTML 报告同目录 (artifact 目录),文件名 `{task_id}.jsonl`
pub struct LocalEventLogger {
    /// task_id (用于日志)
    task_id: String,
    /// 文件 writer,写入失败后设为 None (后续事件跳过文件写入)
    writer: Arc<Mutex<Option<BufWriter<File>>>>,
}

impl LocalEventLogger {
    /// 创建本地 .jsonl 文件,写入 task_started 行
    ///
    /// 文件创建/写入失败时返回 None (调用方降级为无日志)
    pub fn new(task_id: &str, started: &TaskStartedEvent, artifact_dir: &Path) -> Option<Self> {
        let file_path = artifact_dir.join(format!("{task_id}.jsonl"));
        // 文件打开失败时返回 None (调用方降级为无日志)
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .map_err(|e| {
                tracing::warn!(
                    task_id = task_id,
                    path = %file_path.display(),
                    error = %e,
                    "本地事件文件创建失败,后续事件跳过文件写入"
                );
            })
            .ok()?;

        let logger = Self {
            task_id: task_id.to_string(),
            writer: Arc::new(Mutex::new(Some(BufWriter::new(file)))),
        };

        // 写入 task_started 行
        let entry = EventLogEntry::TaskStarted {
            data: started.clone(),
        };
        logger.write_entry(&entry);

        Some(logger)
    }

    /// 追加一行 execution 事件
    pub fn log_event(&self, event: &ExecutionEvent) {
        let entry = EventLogEntry::Execution {
            task_id: self.task_id.clone(),
            event: event.clone(),
        };
        self.write_entry(&entry);
    }

    /// 写入 task_finished 行并关闭文件
    pub fn finish(&self, finished: &TaskFinishedEvent) {
        let entry = EventLogEntry::TaskFinished {
            data: finished.clone(),
        };
        self.write_entry(&entry);

        // flush 并关闭文件
        if let Ok(mut guard) = self.writer.lock() {
            if let Some(w) = guard.as_mut() {
                if let Err(e) = w.flush() {
                    tracing::warn!(task_id = %self.task_id, error = %e, "事件文件 flush 失败");
                }
            }
            // 设为 None 标记已关闭
            *guard = None;
        }
    }

    /// 内部: 写入一行 JSON 并 flush (保证落盘)
    fn write_entry(&self, entry: &EventLogEntry) {
        let Ok(mut guard) = self.writer.lock() else {
            return; // 锁中毒,跳过
        };
        let Some(writer) = guard.as_mut() else {
            return; // writer 已关闭或创建失败,跳过
        };
        match serde_json::to_string(entry) {
            Ok(line) => {
                if let Err(e) = writeln!(writer, "{line}") {
                    tracing::warn!(task_id = %self.task_id, error = %e, "事件文件写入失败");
                    // 写入失败,设为 None 避免后续重复失败
                    *guard = None;
                    return;
                }
                if let Err(e) = writer.flush() {
                    tracing::warn!(task_id = %self.task_id, error = %e, "事件文件 flush 失败");
                    *guard = None;
                    return;
                }
            }
            Err(e) => {
                tracing::warn!(task_id = %self.task_id, error = %e, "事件序列化失败");
            }
        }
    }
}

// ============================================================
// EventSender
// ============================================================

/// 事件推送器 (仅 dashboard.enabled=true 时创建)
///
/// 内部通过 mpsc channel 接收事件,后台 task 批量 POST 到 dashboard。
/// 推送失败不影响主任务执行 (tracing::warn 记录)。
pub struct EventSender {
    /// channel 发送端 (send() 写入此 channel)
    tx: Option<mpsc::Sender<ExecutionEvent>>,
    /// task_id
    task_id: String,
    /// 后台 task handle (finish 时 await)
    handle: Option<JoinHandle<()>>,
}

impl EventSender {
    /// 创建推送器并启动后台批量推送 task
    ///
    /// 注意: 此方法不发送 TaskStartedEvent,需单独调用 `send_start()`。
    pub fn new(config: &DashboardConfig, task_id: &str) -> Self {
        let (tx, rx) = mpsc::channel::<ExecutionEvent>(config.batch_size * 2);

        let batch_config = config.clone();
        let batch_task_id = task_id.to_string();

        let handle = tokio::spawn(async move {
            background_batch_loop(rx, &batch_config, &batch_task_id).await;
        });

        Self {
            tx: Some(tx),
            task_id: task_id.to_string(),
            handle: Some(handle),
        }
    }

    /// 推送 TaskStartedEvent (POST /api/events/start)
    ///
    /// 失败时 tracing::warn 记录,不返回错误 (不影响主任务)。
    pub async fn send_start(&self, config: &DashboardConfig, event: &TaskStartedEvent) {
        let url = format!("{}/api/events/start", config.url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        post_with_retry(&client, &url, &config.token, event).await;
    }

    /// 推送单条事件 (非阻塞,写入 channel)
    ///
    /// channel 满时丢弃事件并 tracing::warn。
    pub fn send(&self, event: ExecutionEvent) {
        if let Some(tx) = &self.tx {
            if tx.try_send(event).is_err() {
                tracing::warn!(task_id = %self.task_id, "EventSender channel 满,丢弃事件");
            }
        }
    }

    /// 任务结束时 flush 并发送 TaskFinishedEvent (POST /api/events/finish)
    ///
    /// 1. 关闭 channel (drop tx) 触发后台 task flush 剩余事件
    /// 2. 等待后台 task 完成
    /// 3. POST /api/events/finish
    ///
    /// 失败时 tracing::warn 记录,不返回错误 (不影响主任务)。
    pub async fn finish(mut self, config: &DashboardConfig, event: TaskFinishedEvent) {
        // 关闭 channel,触发后台 task flush
        self.tx.take();

        // 等待后台 task 完成
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }

        // POST /api/events/finish
        let url = format!("{}/api/events/finish", config.url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        post_with_retry(&client, &url, &config.token, &event).await;
    }
}

// ============================================================
// 后台批量推送逻辑
// ============================================================

/// 后台批量推送循环
///
/// 触发条件 (任一满足):
/// - 事件数累积到 batch_size
/// - 距上次检查超过 batch_interval_secs
/// - channel 关闭 (任务结束 flush)
async fn background_batch_loop(
    mut rx: mpsc::Receiver<ExecutionEvent>,
    config: &DashboardConfig,
    task_id: &str,
) {
    let mut batch: Vec<ExecutionEvent> = Vec::with_capacity(config.batch_size);
    let interval = Duration::from_secs(config.batch_interval_secs);
    let client = reqwest::Client::new();

    loop {
        let timeout = tokio::time::sleep(interval);
        tokio::pin!(timeout);

        tokio::select! {
            maybe_event = rx.recv() => {
                match maybe_event {
                    Some(event) => {
                        batch.push(event);
                        if batch.len() >= config.batch_size {
                            flush_batch(&client, config, task_id, &mut batch).await;
                        }
                    }
                    None => {
                        // channel 关闭, flush 剩余事件并退出
                        if !batch.is_empty() {
                            flush_batch(&client, config, task_id, &mut batch).await;
                        }
                        break;
                    }
                }
            }
            _ = &mut timeout => {
                // 超时触发,如有事件则推送
                if !batch.is_empty() {
                    flush_batch(&client, config, task_id, &mut batch).await;
                }
            }
        }
    }
}

/// 批量推送一批事件 (POST /api/events/batch)
///
/// 失败时指数退避重试 (1s/2s/4s/8s/16s,共 6 次尝试),仍失败则丢弃。
async fn flush_batch(
    client: &reqwest::Client,
    config: &DashboardConfig,
    task_id: &str,
    batch: &mut Vec<ExecutionEvent>,
) {
    if batch.is_empty() {
        return;
    }

    let events = std::mem::take(batch);
    let request = BatchEventsRequest {
        task_id: task_id.to_string(),
        events,
    };

    let url = format!("{}/api/events/batch", config.url.trim_end_matches('/'));
    post_with_retry(client, &url, &config.token, &request).await;
}

/// 带指数退避重试的 POST 请求
///
/// 重试延迟: 1s/2s/4s/8s/16s (初始 + 5 次重试 = 6 次尝试)
async fn post_with_retry<T: serde::Serialize>(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    body: &T,
) {
    let delays = [1u64, 2, 4, 8, 16];
    let mut last_error = String::new();

    for attempt in 0..=5 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(delays[attempt - 1])).await;
        }
        match client
            .post(url)
            .header("X-Devnpc-Token", token)
            .json(body)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(attempt, url = %url, "推送成功");
                return;
            }
            Ok(resp) => {
                last_error = format!("HTTP {}", resp.status());
                tracing::warn!(attempt, url = %url, status = %resp.status(), "推送失败");
            }
            Err(e) => {
                last_error = e.to_string();
                tracing::warn!(attempt, url = %url, error = %e, "推送失败");
            }
        }
    }

    tracing::warn!(url = %url, error = %last_error, "推送重试耗尽,放弃");
}

#[cfg(test)]
mod local_event_logger_tests {
    use super::*;
    use devnpc_core::report::event_schema::{TaskFinishedEvent, TaskStatus};

    fn make_started(task_id: &str) -> TaskStartedEvent {
        TaskStartedEvent {
            task_id: task_id.to_string(),
            project: "test-group/test-project".to_string(),
            mr_iid: Some(42),
            pipeline_id: Some(100),
            task_description: "测试任务".to_string(),
            task_kind: "mr_comment".to_string(),
            started_at: "2026-08-03T10:00:00Z".to_string(),
            model: "deepseek-chat".to_string(),
        }
    }

    fn make_finished(task_id: &str) -> TaskFinishedEvent {
        TaskFinishedEvent {
            task_id: task_id.to_string(),
            status: TaskStatus::Success,
            duration_secs: 45,
            total_tokens: 12000,
            estimated_cost_usd: 0.05,
            mr_url: Some("https://gitlab.com/mr/42".to_string()),
            ci_url: Some("https://gitlab.com/pipeline/100".to_string()),
            summary: "已修复".to_string(),
            error: None,
            finished_at: "2026-08-03T10:01:00Z".to_string(),
        }
    }

    #[test]
    fn new_creates_file_with_task_started_line() {
        let dir = tempfile::tempdir().unwrap();
        let task_id = "test-new-creates";
        let started = make_started(task_id);

        let logger = LocalEventLogger::new(task_id, &started, dir.path());
        assert!(logger.is_some());

        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        assert!(file_path.exists(), "事件文件应已创建");

        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(
            content.contains("task_started"),
            "首行应包含 task_started, 实际: {content}"
        );
        assert!(content.contains(task_id));
    }

    #[test]
    fn log_event_appends_execution_line() {
        let dir = tempfile::tempdir().unwrap();
        let task_id = "test-log-event";
        let started = make_started(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path()).unwrap();

        let event = ExecutionEvent::ToolCall {
            name: "read_file".to_string(),
            success: true,
            latency_ms: 50,
            detail: "src/main.rs".to_string(),
        };
        logger.log_event(&event);

        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "应有 2 行: task_started + execution");
        assert!(lines[1].contains("execution"));
        assert!(lines[1].contains("tool_call"));
        assert!(lines[1].contains("read_file"));
    }

    #[test]
    fn finish_writes_task_finished_and_closes() {
        let dir = tempfile::tempdir().unwrap();
        let task_id = "test-finish";
        let started = make_started(task_id);
        let finished = make_finished(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path()).unwrap();

        let event = ExecutionEvent::LlmCall {
            iteration: 1,
            prompt_tokens: 100,
            completion_tokens: 50,
            latency_ms: 500,
        };
        logger.log_event(&event);
        logger.finish(&finished);

        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3, "应有 3 行: started + execution + finished");
        assert!(lines[2].contains("task_finished"));
        assert!(lines[2].contains("success"));
    }

    #[test]
    fn non_existent_dir_doesnt_panic() {
        let task_id = "test-no-dir";
        let started = make_started(task_id);
        // 基于 tempdir 构造一个确定不存在的子目录 (跨平台)
        let parent = tempfile::tempdir().unwrap();
        let nonexistent = parent.path().join("nested/sub/dir/that/does/not/exist");

        // 不应 panic
        let logger = LocalEventLogger::new(task_id, &started, &nonexistent);
        assert!(logger.is_none(), "目录不存在时应返回 None");
    }

    #[test]
    fn generated_jsonl_is_parseable() {
        let dir = tempfile::tempdir().unwrap();
        let task_id = "test-parseable";
        let started = make_started(task_id);
        let finished = make_finished(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path()).unwrap();

        let event1 = ExecutionEvent::LlmCall {
            iteration: 1,
            prompt_tokens: 500,
            completion_tokens: 200,
            latency_ms: 1500,
        };
        let event2 = ExecutionEvent::SopStep {
            step: "analyze".to_string(),
            status: devnpc_core::report::event_schema::SopStepStatus::Completed,
            note: None,
        };
        logger.log_event(&event1);
        logger.log_event(&event2);
        logger.finish(&finished);

        // 读取并解析每行
        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 4);

        // 每行都应能解析为 EventLogEntry
        for (i, line) in lines.iter().enumerate() {
            let result: Result<EventLogEntry, _> = serde_json::from_str(line);
            assert!(result.is_ok(), "第 {i} 行解析失败: {line}");
        }

        // 验证第一行是 TaskStarted
        let first: EventLogEntry = serde_json::from_str(lines[0]).unwrap();
        assert!(matches!(first, EventLogEntry::TaskStarted { .. }));

        // 验证最后一行是 TaskFinished
        let last: EventLogEntry = serde_json::from_str(lines[3]).unwrap();
        assert!(matches!(last, EventLogEntry::TaskFinished { .. }));
    }

    #[test]
    fn multiple_events_append_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let task_id = "test-order";
        let started = make_started(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path()).unwrap();

        for i in 0..5 {
            logger.log_event(&ExecutionEvent::LlmCall {
                iteration: i,
                prompt_tokens: 100,
                completion_tokens: 50,
                latency_ms: 500,
            });
        }

        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        // 1 started + 5 execution = 6 行
        assert_eq!(lines.len(), 6);
    }
}

#[cfg(test)]
mod event_sender_tests {
    use super::*;
    use devnpc_core::report::event_schema::{
        TaskFinishedEvent, TaskStartedEvent, TaskStatus,
    };
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_config(url: String) -> DashboardConfig {
        DashboardConfig {
            enabled: true,
            url,
            token: "test-token".to_string(),
            batch_size: 2, // 小阈值便于测试
            batch_interval_secs: 60,  // 大阈值,避免时间触发干扰
            local_event_log: false,
        }
    }

    fn make_started(task_id: &str) -> TaskStartedEvent {
        TaskStartedEvent {
            task_id: task_id.to_string(),
            project: "test".to_string(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "test".to_string(),
            task_kind: "manual".to_string(),
            started_at: "2026-08-03T10:00:00Z".to_string(),
            model: "test-model".to_string(),
        }
    }

    fn make_finished(task_id: &str) -> TaskFinishedEvent {
        TaskFinishedEvent {
            task_id: task_id.to_string(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 1000,
            estimated_cost_usd: 0.01,
            mr_url: None,
            ci_url: None,
            summary: "done".to_string(),
            error: None,
            finished_at: "2026-08-03T10:01:00Z".to_string(),
        }
    }

    fn make_llm_event(iteration: u32) -> ExecutionEvent {
        ExecutionEvent::LlmCall {
            iteration,
            prompt_tokens: 100,
            completion_tokens: 50,
            latency_ms: 500,
        }
    }

    #[tokio::test]
    async fn send_start_posts_to_dashboard() {
        let server = MockServer::start().await;
        let config = make_config(server.uri());

        Mock::given(method("POST"))
            .and(path("/api/events/start"))
            .and(header("X-Devnpc-Token", "test-token"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-start-1");
        let started = make_started("task-start-1");
        sender.send_start(&config, &started).await;

        // finish 关闭后台 task (避免泄漏) - mount 通用 mock 接收 finish 和 batch
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        sender.finish(&config, make_finished("task-start-1")).await;
    }

    #[tokio::test]
    async fn batch_triggers_on_size_threshold() {
        let server = MockServer::start().await;
        let config = make_config(server.uri());

        Mock::given(method("POST"))
            .and(path("/api/events/batch"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-batch-size");
        // batch_size=2,发送 2 条触发一次批量推送
        sender.send(make_llm_event(1));
        sender.send(make_llm_event(2));

        // 等待后台 task 处理
        tokio::time::sleep(Duration::from_millis(500)).await;

        // mount finish mock 后再 finish,避免 finish POST 失败重试阻塞
        Mock::given(method("POST"))
            .and(path("/api/events/finish"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        sender.finish(&config, make_finished("task-batch-size")).await;
    }

    #[tokio::test]
    async fn batch_triggers_on_timeout() {
        let server = MockServer::start().await;
        let mut config = make_config(server.uri());
        config.batch_size = 100; // 大阈值,不会因数量触发
        config.batch_interval_secs = 1; // 1 秒后触发

        Mock::given(method("POST"))
            .and(path("/api/events/batch"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-batch-timeout");
        sender.send(make_llm_event(1));

        // 等待超时触发 (1 秒 + 余量)
        tokio::time::sleep(Duration::from_millis(1500)).await;

        Mock::given(method("POST"))
            .and(path("/api/events/finish"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        sender.finish(&config, make_finished("task-batch-timeout")).await;
    }

    #[tokio::test(start_paused = true)]
    async fn retry_on_failure_with_backoff() {
        let server = MockServer::start().await;
        let config = make_config(server.uri());

        // 前 2 次返回 500,第 3 次返回 200
        Mock::given(method("POST"))
            .and(path("/api/events/batch"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/events/batch"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-retry");
        sender.send(make_llm_event(1));
        sender.send(make_llm_event(2)); // 触发 batch

        // 等待重试完成 (start_paused 使 sleep 立即返回)
        tokio::time::sleep(Duration::from_millis(100)).await;

        Mock::given(method("POST"))
            .and(path("/api/events/finish"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        sender.finish(&config, make_finished("task-retry")).await;
    }

    #[tokio::test(start_paused = true)]
    async fn discard_after_max_retries() {
        let server = MockServer::start().await;
        let config = make_config(server.uri());

        // 所有请求都返回 500
        Mock::given(method("POST"))
            .and(path("/api/events/batch"))
            .respond_with(ResponseTemplate::new(500))
            // 期望 6 次 (初始 + 5 次重试)
            .expect(6)
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-discard");
        sender.send(make_llm_event(1));
        sender.send(make_llm_event(2)); // 触发 batch

        // 等待所有重试完成
        tokio::time::sleep(Duration::from_millis(500)).await;

        Mock::given(method("POST"))
            .and(path("/api/events/finish"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        sender.finish(&config, make_finished("task-discard")).await;
    }

    #[tokio::test]
    async fn finish_posts_task_finished() {
        let server = MockServer::start().await;
        let config = make_config(server.uri());

        Mock::given(method("POST"))
            .and(path("/api/events/finish"))
            .and(header("X-Devnpc-Token", "test-token"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-finish");
        sender.finish(&config, make_finished("task-finish")).await;
    }

    #[tokio::test]
    async fn send_does_not_block_when_channel_full() {
        let server = MockServer::start().await;
        let mut config = make_config(server.uri());
        config.batch_size = 1; // 极小 channel
        config.batch_interval_secs = 3600; // 不触发时间推送

        // mount 通用 200 响应避免 finish/batch 推送阻塞重试
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-full");

        // 填满 channel (batch_size * 2 = 2 缓冲)
        for i in 0..10 {
            sender.send(make_llm_event(i)); // 不应 panic
        }

        // finish 会 flush 剩余 (mock 已 mount 200)
        sender.finish(&config, make_finished("task-full")).await;
    }
}
