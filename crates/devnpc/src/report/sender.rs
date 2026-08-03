//! Dashboard 事件推送器与本地事件记录器 (spec §4.2)
//!
//! 两个独立组件:
//! - `LocalEventLogger`: 本地 .jsonl 文件记录 (兜底机制,独立于推送)
//! - `EventSender`: channel + 异步批量 POST 推送 (仅 dashboard.enabled=true 时创建)

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use devnpc_core::report::event_schema::{
    ExecutionEvent, EventLogEntry, TaskFinishedEvent, TaskStartedEvent,
};

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
