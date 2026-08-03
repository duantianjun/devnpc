//! RealtimeHub 实时事件中心
//!
//! 内存环形缓冲 (VecDeque 容量上限) + broadcast 广播。
//! subscribe() 先回放缓冲历史,再推送实时事件。

use std::collections::{HashSet, VecDeque};

use devnpc_core::report::event_schema::ExecutionEvent;
use futures::stream::{self, Stream};
use futures::StreamExt;
use tokio::sync::{broadcast, RwLock};
use tokio_stream::wrappers::BroadcastStream;

/// 推送到 SSE 订阅者的事件
#[derive(Debug, Clone, serde::Serialize)]
pub struct RealtimeEvent {
    pub task_id: String,
    pub event: ExecutionEvent,
    pub timestamp: String,
}

/// 实时事件中心
pub struct RealtimeHub {
    /// 环形缓冲 (最近 N 条事件)
    buffer: RwLock<VecDeque<RealtimeEvent>>,
    /// broadcast 发送端
    tx: broadcast::Sender<RealtimeEvent>,
    /// 缓冲容量
    buffer_capacity: usize,
    /// 当前 running 任务集合
    running_tasks: RwLock<HashSet<String>>,
}

impl RealtimeHub {
    pub fn new(buffer_capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(buffer_capacity.max(16));
        Self {
            buffer: RwLock::new(VecDeque::with_capacity(buffer_capacity)),
            tx,
            buffer_capacity,
            running_tasks: RwLock::new(HashSet::new()),
        }
    }

    /// 标记任务启动 (加入 running 集合)
    pub async fn task_started(&self, task_id: &str) {
        self.running_tasks.write().await.insert(task_id.to_string());
    }

    /// 标记任务结束 (移出 running 集合)
    pub async fn task_finished(&self, task_id: &str) {
        self.running_tasks.write().await.remove(task_id);
    }

    /// 推送事件到缓冲与所有订阅者
    pub async fn push_events(&self, task_id: &str, events: &[ExecutionEvent]) {
        let now = chrono::Utc::now().to_rfc3339();
        let mut buf = self.buffer.write().await;
        for ev in events {
            let re = RealtimeEvent {
                task_id: task_id.to_string(),
                event: ev.clone(),
                timestamp: now.clone(),
            };
            // 环形缓冲: 满则淘汰最旧
            if buf.len() >= self.buffer_capacity {
                buf.pop_front();
            }
            buf.push_back(re.clone());
            // broadcast: 忽略无订阅者的错误
            let _ = self.tx.send(re);
        }
    }

    /// 订阅事件流: 先回放缓冲历史,再接收实时事件
    pub async fn subscribe(&self) -> impl Stream<Item = RealtimeEvent> + use<> {
        // 克隆历史快照
        let history: Vec<RealtimeEvent> = {
            let buf = self.buffer.read().await;
            buf.iter().cloned().collect()
        };
        let rx = self.tx.subscribe();
        let live = BroadcastStream::new(rx).filter_map(|r| async move { r.ok() });
        stream::iter(history).chain(live)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devnpc_core::report::event_schema::ExecutionEvent;

    fn llm(i: u32) -> ExecutionEvent {
        ExecutionEvent::LlmCall {
            iteration: i,
            prompt_tokens: 10,
            completion_tokens: 5,
            latency_ms: 100,
        }
    }

    #[tokio::test]
    async fn push_events_stores_in_buffer() {
        let hub = RealtimeHub::new(100);
        hub.push_events("t1", &[llm(1), llm(2)]).await;
        let buf = hub.buffer.read().await;
        assert_eq!(buf.len(), 2);
        assert_eq!(buf[0].task_id, "t1");
    }

    #[tokio::test]
    async fn buffer_evicts_oldest_when_full() {
        let hub = RealtimeHub::new(2);
        hub.push_events("t1", &[llm(1)]).await;
        hub.push_events("t1", &[llm(2)]).await;
        hub.push_events("t1", &[llm(3)]).await;
        let buf = hub.buffer.read().await;
        assert_eq!(buf.len(), 2);
        // 最旧的 llm(1) 被淘汰
        match &buf[0].event {
            ExecutionEvent::LlmCall { iteration, .. } => assert_eq!(*iteration, 2),
            _ => panic!(),
        }
    }

    #[tokio::test]
    async fn task_started_and_finished() {
        let hub = RealtimeHub::new(100);
        hub.task_started("t1").await;
        assert!(hub.running_tasks.read().await.contains("t1"));
        hub.task_finished("t1").await;
        assert!(!hub.running_tasks.read().await.contains("t1"));
    }

    #[tokio::test]
    async fn subscribe_receives_history_then_live() {
        let hub = std::sync::Arc::new(RealtimeHub::new(100));
        // 先推一条历史
        hub.push_events("t1", &[llm(1)]).await;
        // 订阅 (应包含历史)
        let stream = hub.subscribe().await;
        tokio::pin!(stream);
        // 读取历史
        let first = stream.next().await;
        assert!(first.is_some());
        // 推送实时
        hub.push_events("t1", &[llm(2)]).await;
        let second = stream.next().await;
        assert!(second.is_some());
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let hub = std::sync::Arc::new(RealtimeHub::new(100));
        let s1 = hub.subscribe().await;
        let s2 = hub.subscribe().await;
        tokio::pin!(s1);
        tokio::pin!(s2);
        hub.push_events("t1", &[llm(1)]).await;
        assert!(s1.next().await.is_some());
        assert!(s2.next().await.is_some());
    }
}
