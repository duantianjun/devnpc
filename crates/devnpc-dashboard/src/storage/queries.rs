//! Storage 结构与数据库查询
//!
//! Arc<Mutex<Connection>> 串行化写;WAL 模式下读不阻塞写。

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use devnpc_core::report::event_schema::{
    ExecutionEvent, TaskFinishedEvent, TaskStartedEvent,
};

use crate::error::Result;
use crate::storage::schema;

// ============================================================
// 行类型
// ============================================================

/// tasks 表行映射
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskRow {
    pub task_id: String,
    pub project: String,
    pub mr_iid: Option<u64>,
    pub pipeline_id: Option<u64>,
    pub task_description: String,
    pub task_kind: String,
    pub model: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_secs: Option<u64>,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: f64,
    pub mr_url: Option<String>,
    pub ci_url: Option<String>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub ci_retries: u64,
}

/// events 表行映射
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventRow {
    pub id: i64,
    pub task_id: String,
    pub seq: i64,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}

/// 任务列表过滤条件
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub status: Option<String>,
    pub project: Option<String>,
}

/// 任务列表响应 (带分页元信息)
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskListResponse {
    pub tasks: Vec<TaskRow>,
    pub total: usize,
    pub page: usize,
    pub size: usize,
}

/// 趋势统计单点
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrendPoint {
    pub date: String,
    pub total: u64,
    pub success: u64,
    pub failed: u64,
    pub avg_duration_secs: f64,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
}

/// 趋势统计结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrendsData {
    pub days: u32,
    pub points: Vec<TrendPoint>,
}

/// 成本聚合桶
#[derive(Debug, Clone, serde::Serialize)]
pub struct CostBucket {
    pub key: String,
    pub total_cost_usd: f64,
    pub total_tokens: u64,
    pub task_count: u64,
}

/// CI 自愈统计
#[derive(Debug, Clone, serde::Serialize)]
pub struct CiStats {
    pub total_failed: u64,
    pub auto_healed: u64,
    pub heal_rate: f64,
    pub avg_retries: f64,
    pub failed_tasks: Vec<TaskRow>,
}

/// SOP 偏离记录行
#[derive(Debug, Clone, serde::Serialize)]
pub struct SopDeviationRow {
    pub id: i64,
    pub task_id: String,
    pub step: String,
    pub note: Option<String>,
    pub created_at: String,
}

// ============================================================
// Storage
// ============================================================

/// SQLite 存储层
#[derive(Clone)]
pub struct Storage {
    conn: Arc<Mutex<Connection>>,
}

impl Storage {
    /// 打开文件数据库,开启 WAL 并执行 schema 迁移
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        // WAL 模式: 读不阻塞写
        conn.pragma_update(None, "journal_mode", "WAL")?;
        schema::init_db(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 打开内存数据库 (测试用)
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::init_db(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 创建任务记录,状态=running (重复 task_id 返回 TaskConflict)
    pub fn start_task(&self, e: &TaskStartedEvent) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        if self.task_exists_locked(&conn, &e.task_id)? {
            return Err(crate::error::DashboardError::TaskConflict(format!(
                "任务 {} 已存在",
                e.task_id
            )));
        }
        conn.execute(
            "INSERT INTO tasks (task_id, project, mr_iid, pipeline_id, task_description, task_kind, model, status, started_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', ?8)",
            rusqlite::params![
                e.task_id, e.project, e.mr_iid, e.pipeline_id,
                e.task_description, e.task_kind, e.model, e.started_at,
            ],
        )?;
        Ok(())
    }

    /// 批量写入执行事件 (task_id 不存在返回 TaskNotFound)
    pub fn insert_events(&self, task_id: &str, events: &[ExecutionEvent]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        if !self.task_exists_locked(&conn, task_id)? {
            return Err(crate::error::DashboardError::TaskNotFound(task_id.into()));
        }
        // 取当前最大 seq
        let max_seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM events WHERE task_id = ?1",
                rusqlite::params![task_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let now = chrono::Utc::now().to_rfc3339();
        let mut seq = max_seq;
        for ev in events {
            seq += 1;
            let payload = serde_json::to_string(ev)?;
            let event_type = match ev {
                ExecutionEvent::LlmCall { .. } => "llm_call",
                ExecutionEvent::ToolCall { .. } => "tool_call",
                ExecutionEvent::SopStep { .. } => "sop_step",
                ExecutionEvent::CiStatus { .. } => "ci_status",
                ExecutionEvent::TeamHandoff { .. } => "team_handoff",
            };
            conn.execute(
                "INSERT INTO events (task_id, seq, event_type, payload, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![task_id, seq, event_type, payload, now],
            )?;
        }
        Ok(())
    }

    /// 任务结束: 更新状态/汇总字段,聚合 tokens 与 ci_retries,写入 sop_deviations
    pub fn finish_task(&self, e: &TaskFinishedEvent) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        if !self.task_exists_locked(&conn, &e.task_id)? {
            return Err(crate::error::DashboardError::TaskNotFound(e.task_id.clone()));
        }
        if self.task_is_finished_locked(&conn, &e.task_id)? {
            return Err(crate::error::DashboardError::TaskConflict(format!(
                "任务 {} 已结束",
                e.task_id
            )));
        }
        // 聚合 input/output tokens (从 llm_call 事件)
        let (input_tokens, output_tokens): (u64, u64) = conn
            .query_row(
                "SELECT \
                   COALESCE(SUM(json_extract(payload, '$.prompt_tokens')), 0), \
                   COALESCE(SUM(json_extract(payload, '$.completion_tokens')), 0) \
                 FROM events WHERE task_id = ?1 AND event_type = 'llm_call'",
                rusqlite::params![e.task_id],
                |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
            )
            .unwrap_or((0, 0));
        // 聚合 ci_retries (ci_status 事件的最大 attempt)
        let ci_retries: u64 = conn
            .query_row(
                "SELECT COALESCE(MAX(json_extract(payload, '$.attempt')), 0) \
                 FROM events WHERE task_id = ?1 AND event_type = 'ci_status'",
                rusqlite::params![e.task_id],
                |r| Ok(r.get::<_, i64>(0)? as u64),
            )
            .unwrap_or(0);
        let status_str = match e.status {
            devnpc_core::report::event_schema::TaskStatus::Success => "success",
            devnpc_core::report::event_schema::TaskStatus::Failed => "failed",
            devnpc_core::report::event_schema::TaskStatus::CiFailed => "ci_failed",
            devnpc_core::report::event_schema::TaskStatus::Timeout => "timeout",
        };
        conn.execute(
            "UPDATE tasks SET status = ?1, finished_at = ?2, duration_secs = ?3, \
             total_tokens = ?4, input_tokens = ?5, output_tokens = ?6, \
             estimated_cost_usd = ?7, mr_url = ?8, ci_url = ?9, summary = ?10, \
             error = ?11, ci_retries = ?12 WHERE task_id = ?13",
            rusqlite::params![
                status_str,
                e.finished_at,
                e.duration_secs as i64,
                e.total_tokens as i64,
                input_tokens as i64,
                output_tokens as i64,
                e.estimated_cost_usd,
                e.mr_url,
                e.ci_url,
                e.summary,
                e.error,
                ci_retries as i64,
                e.task_id,
            ],
        )?;
        // 写入 sop_deviations (从 sop_step 事件中 status=deviated 的)
        let now = chrono::Utc::now().to_rfc3339();
        let mut stmt = conn.prepare(
            "SELECT json_extract(payload, '$.step'), json_extract(payload, '$.note') \
             FROM events WHERE task_id = ?1 AND event_type = 'sop_step' \
             AND json_extract(payload, '$.status') = 'deviated'",
        )?;
        let deviations: Vec<(Option<String>, Option<String>)> = stmt
            .query_map(rusqlite::params![e.task_id], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        for (step, note) in deviations {
            conn.execute(
                "INSERT INTO sop_deviations (task_id, step, note, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![e.task_id, step, note, now],
            )?;
        }
        Ok(())
    }

    /// 任务是否存在
    pub fn task_exists(&self, task_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        self.task_exists_locked(&conn, task_id)
    }

    /// 任务是否已结束 (status != running)
    pub fn task_is_finished(&self, task_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        self.task_is_finished_locked(&conn, task_id)
    }

    /// 删除任务及其事件与偏离记录 (覆盖导入时使用)
    pub fn delete_task(&self, task_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM sop_deviations WHERE task_id = ?1", rusqlite::params![task_id])?;
        conn.execute("DELETE FROM events WHERE task_id = ?1", rusqlite::params![task_id])?;
        conn.execute("DELETE FROM tasks WHERE task_id = ?1", rusqlite::params![task_id])?;
        Ok(())
    }

    /// 查询单个任务
    pub fn get_task(&self, task_id: &str) -> Result<Option<TaskRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT task_id, project, mr_iid, pipeline_id, task_description, task_kind, \
             model, status, started_at, finished_at, duration_secs, total_tokens, \
             input_tokens, output_tokens, estimated_cost_usd, mr_url, ci_url, summary, \
             error, ci_retries FROM tasks WHERE task_id = ?1",
        )?;
        let row = stmt
            .query_row(rusqlite::params![task_id], |r| {
                Ok(TaskRow {
                    task_id: r.get(0)?,
                    project: r.get(1)?,
                    mr_iid: r.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                    pipeline_id: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                    task_description: r.get(4)?,
                    task_kind: r.get(5)?,
                    model: r.get(6)?,
                    status: r.get(7)?,
                    started_at: r.get(8)?,
                    finished_at: r.get(9)?,
                    duration_secs: r.get::<_, Option<i64>>(10)?.map(|v| v as u64),
                    total_tokens: r.get::<_, i64>(11)? as u64,
                    input_tokens: r.get::<_, i64>(12)? as u64,
                    output_tokens: r.get::<_, i64>(13)? as u64,
                    estimated_cost_usd: r.get(14)?,
                    mr_url: r.get(15)?,
                    ci_url: r.get(16)?,
                    summary: r.get(17)?,
                    error: r.get(18)?,
                    ci_retries: r.get::<_, i64>(19)? as u64,
                })
            })
            .ok();
        Ok(row)
    }

    /// 查询任务的事件列表 (按 seq 升序)
    pub fn list_events(&self, task_id: &str) -> Result<Vec<EventRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, task_id, seq, event_type, payload, created_at \
             FROM events WHERE task_id = ?1 ORDER BY seq ASC",
        )?;
        let rows: Vec<EventRow> = stmt
            .query_map(rusqlite::params![task_id], |r| {
                let payload_str: String = r.get(4)?;
                let payload: serde_json::Value =
                    serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null);
                Ok(EventRow {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    seq: r.get(2)?,
                    event_type: r.get(3)?,
                    payload,
                    created_at: r.get(5)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// 分页 + 过滤查询任务列表 (按 started_at 倒序)
    pub fn list_tasks(&self, page: usize, size: usize, filter: &TaskFilter) -> Result<TaskListResponse> {
        let conn = self.conn.lock().unwrap();
        let page = if page == 0 { 1 } else { page };
        let size = if size == 0 { 20 } else { size };
        let offset = ((page - 1) * size) as i64;

        // 动态拼接 WHERE
        let mut where_clauses: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(s) = &filter.status {
            where_clauses.push(format!("status = ?{}", where_clauses.len() + 1));
            params.push(Box::new(s.clone()));
        }
        if let Some(p) = &filter.project {
            where_clauses.push(format!("project = ?{}", where_clauses.len() + 1));
            params.push(Box::new(p.clone()));
        }
        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        // 总数
        let count_sql = format!("SELECT COUNT(*) FROM tasks {}", where_sql);
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let total: i64 = conn.query_row(&count_sql, param_refs.as_slice(), |r| r.get(0))?;

        // 分页数据
        let list_sql = format!(
            "SELECT task_id, project, mr_iid, pipeline_id, task_description, task_kind, \
             model, status, started_at, finished_at, duration_secs, total_tokens, \
             input_tokens, output_tokens, estimated_cost_usd, mr_url, ci_url, summary, \
             error, ci_retries FROM tasks {} ORDER BY started_at DESC LIMIT ?{} OFFSET ?{}",
            where_sql,
            params.len() + 1,
            params.len() + 2,
        );
        let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = params;
        all_params.push(Box::new(size as i64));
        all_params.push(Box::new(offset));
        let all_refs: Vec<&dyn rusqlite::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&list_sql)?;
        let tasks: Vec<TaskRow> = stmt
            .query_map(all_refs.as_slice(), |r| {
                Ok(TaskRow {
                    task_id: r.get(0)?,
                    project: r.get(1)?,
                    mr_iid: r.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                    pipeline_id: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                    task_description: r.get(4)?,
                    task_kind: r.get(5)?,
                    model: r.get(6)?,
                    status: r.get(7)?,
                    started_at: r.get(8)?,
                    finished_at: r.get(9)?,
                    duration_secs: r.get::<_, Option<i64>>(10)?.map(|v| v as u64),
                    total_tokens: r.get::<_, i64>(11)? as u64,
                    input_tokens: r.get::<_, i64>(12)? as u64,
                    output_tokens: r.get::<_, i64>(13)? as u64,
                    estimated_cost_usd: r.get(14)?,
                    mr_url: r.get(15)?,
                    ci_url: r.get(16)?,
                    summary: r.get(17)?,
                    error: r.get(18)?,
                    ci_retries: r.get::<_, i64>(19)? as u64,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(TaskListResponse {
            tasks,
            total: total as usize,
            page,
            size,
        })
    }

    /// 趋势统计 (按天聚合最近 N 天)
    pub fn trends(&self, days: u32) -> Result<TrendsData> {
        let conn = self.conn.lock().unwrap();
        let modifier = format!("-{} days", days);
        let mut stmt = conn.prepare(
            "SELECT date(started_at) as d, COUNT(*), \
             SUM(CASE WHEN status='success' THEN 1 ELSE 0 END), \
             SUM(CASE WHEN status IN ('failed','ci_failed','timeout') THEN 1 ELSE 0 END), \
             AVG(duration_secs), \
             SUM(total_tokens), \
             SUM(estimated_cost_usd) \
             FROM tasks WHERE started_at >= datetime('now', ?1) \
             GROUP BY d ORDER BY d ASC",
        )?;
        let points: Vec<TrendPoint> = stmt
            .query_map(rusqlite::params![modifier], |r| {
                Ok(TrendPoint {
                    date: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    total: r.get::<_, i64>(1)? as u64,
                    success: r.get::<_, i64>(2)? as u64,
                    failed: r.get::<_, i64>(3)? as u64,
                    avg_duration_secs: r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                    total_tokens: r.get::<_, i64>(5)? as u64,
                    total_cost_usd: r.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(TrendsData { days, points })
    }

    /// 成本聚合 (group_by: project/model/kind)
    pub fn cost_breakdown(&self, group_by: &str) -> Result<Vec<CostBucket>> {
        let column = match group_by {
            "project" => "project",
            "model" => "model",
            "kind" => "task_kind",
            _ => {
                return Err(crate::error::DashboardError::ImportFormat(format!(
                    "无效的 group_by: {} (允许 project/model/kind)",
                    group_by
                )))
            }
        };
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} as k, SUM(estimated_cost_usd), SUM(total_tokens), COUNT(*) \
             FROM tasks WHERE status != 'running' GROUP BY k ORDER BY SUM(estimated_cost_usd) DESC",
            column
        );
        let mut stmt = conn.prepare(&sql)?;
        let buckets: Vec<CostBucket> = stmt
            .query_map([], |r| {
                Ok(CostBucket {
                    key: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    total_cost_usd: r.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                    total_tokens: r.get::<_, i64>(2)? as u64,
                    task_count: r.get::<_, i64>(3)? as u64,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(buckets)
    }

    /// CI 自愈统计
    pub fn ci_stats(&self) -> Result<CiStats> {
        let conn = self.conn.lock().unwrap();
        let total_failed: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE status IN ('failed','ci_failed','timeout')",
                [],
                |r| Ok(r.get::<_, i64>(0)? as u64),
            )
            .unwrap_or(0);
        let auto_healed: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE ci_retries > 0 AND status = 'success'",
                [],
                |r| Ok(r.get::<_, i64>(0)? as u64),
            )
            .unwrap_or(0);
        let avg_retries: f64 = conn
            .query_row(
                "SELECT AVG(ci_retries) FROM tasks WHERE ci_retries > 0",
                [],
                |r| Ok(r.get::<_, Option<f64>>(0)?.unwrap_or(0.0)),
            )
            .unwrap_or(0.0);
        let heal_rate = if total_failed + auto_healed == 0 {
            0.0
        } else {
            auto_healed as f64 / (total_failed + auto_healed) as f64
        };
        // 失败任务列表
        let mut stmt = conn.prepare(
            "SELECT task_id, project, mr_iid, pipeline_id, task_description, task_kind, \
             model, status, started_at, finished_at, duration_secs, total_tokens, \
             input_tokens, output_tokens, estimated_cost_usd, mr_url, ci_url, summary, \
             error, ci_retries FROM tasks WHERE status IN ('failed','ci_failed','timeout') \
             ORDER BY started_at DESC LIMIT 100",
        )?;
        let failed_tasks: Vec<TaskRow> = stmt
            .query_map([], |r| {
                Ok(TaskRow {
                    task_id: r.get(0)?,
                    project: r.get(1)?,
                    mr_iid: r.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                    pipeline_id: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                    task_description: r.get(4)?,
                    task_kind: r.get(5)?,
                    model: r.get(6)?,
                    status: r.get(7)?,
                    started_at: r.get(8)?,
                    finished_at: r.get(9)?,
                    duration_secs: r.get::<_, Option<i64>>(10)?.map(|v| v as u64),
                    total_tokens: r.get::<_, i64>(11)? as u64,
                    input_tokens: r.get::<_, i64>(12)? as u64,
                    output_tokens: r.get::<_, i64>(13)? as u64,
                    estimated_cost_usd: r.get(14)?,
                    mr_url: r.get(15)?,
                    ci_url: r.get(16)?,
                    summary: r.get(17)?,
                    error: r.get(18)?,
                    ci_retries: r.get::<_, i64>(19)? as u64,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(CiStats {
            total_failed,
            auto_healed,
            heal_rate,
            avg_retries,
            failed_tasks,
        })
    }

    /// SOP 偏离记录 (最近 100 条)
    pub fn sop_stats(&self) -> Result<Vec<SopDeviationRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, task_id, step, note, created_at \
             FROM sop_deviations ORDER BY created_at DESC LIMIT 100",
        )?;
        let rows: Vec<SopDeviationRow> = stmt
            .query_map([], |r| {
                Ok(SopDeviationRow {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    step: r.get(2)?,
                    note: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    // ---- 内部辅助 (已持有锁) ----

    fn task_exists_locked(&self, conn: &Connection, task_id: &str) -> Result<bool> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    fn task_is_finished_locked(&self, conn: &Connection, task_id: &str) -> Result<bool> {
        let status: Option<String> = conn.query_row(
            "SELECT status FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |r| r.get(0),
        ).ok();
        match status {
            Some(s) => Ok(s != "running"),
            None => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devnpc_core::report::event_schema::{
        CiStatus, ExecutionEvent, SopStepStatus, TaskFinishedEvent, TaskStartedEvent, TaskStatus,
    };

    fn sample_started(task_id: &str) -> TaskStartedEvent {
        TaskStartedEvent {
            task_id: task_id.into(),
            project: "group/proj".into(),
            mr_iid: Some(42),
            pipeline_id: Some(100),
            task_description: "修复 bug".into(),
            task_kind: "mr_comment".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "deepseek-chat".into(),
        }
    }

    #[test]
    fn open_in_memory_succeeds() {
        let s = Storage::open_in_memory();
        assert!(s.is_ok());
    }

    #[test]
    fn open_file_succeeds() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let s = Storage::open(tmp.path().to_str().unwrap());
        assert!(s.is_ok());
    }

    #[test]
    fn storage_is_clone() {
        let s = Storage::open_in_memory().unwrap();
        let _s2 = s.clone();
    }

    #[test]
    fn start_task_inserts_running_row() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        let exists = s.task_exists("t1").unwrap();
        assert!(exists);
        let row = s.get_task("t1").unwrap().unwrap();
        assert_eq!(row.status, "running");
        assert_eq!(row.project, "group/proj");
    }

    #[test]
    fn start_task_duplicate_returns_conflict() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        let err = s.start_task(&sample_started("t1")).unwrap_err();
        assert!(matches!(err, crate::error::DashboardError::TaskConflict(_)));
    }

    #[test]
    fn insert_events_stores_rows() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        let events = vec![
            ExecutionEvent::LlmCall {
                iteration: 1,
                prompt_tokens: 500,
                completion_tokens: 200,
                latency_ms: 1500,
            },
            ExecutionEvent::ToolCall {
                name: "read_file".into(),
                success: true,
                latency_ms: 50,
                detail: "src/main.rs".into(),
            },
        ];
        s.insert_events("t1", &events).unwrap();
        let rows = s.list_events("t1").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].seq, 1);
        assert_eq!(rows[1].seq, 2);
        assert_eq!(rows[0].event_type, "llm_call");
    }

    #[test]
    fn insert_events_unknown_task_returns_not_found() {
        let s = Storage::open_in_memory().unwrap();
        let events = vec![ExecutionEvent::LlmCall {
            iteration: 1,
            prompt_tokens: 10,
            completion_tokens: 5,
            latency_ms: 100,
        }];
        let err = s.insert_events("nope", &events).unwrap_err();
        assert!(matches!(err, crate::error::DashboardError::TaskNotFound(_)));
    }

    #[test]
    fn finish_task_updates_status_and_tokens() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.insert_events(
            "t1",
            &vec![
                ExecutionEvent::LlmCall {
                    iteration: 1,
                    prompt_tokens: 500,
                    completion_tokens: 200,
                    latency_ms: 1500,
                },
                ExecutionEvent::LlmCall {
                    iteration: 2,
                    prompt_tokens: 300,
                    completion_tokens: 100,
                    latency_ms: 1200,
                },
            ],
        )
        .unwrap();
        let finished = TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 45,
            total_tokens: 1100,
            estimated_cost_usd: 0.05,
            mr_url: Some("https://gitlab.com/mr/42".into()),
            ci_url: None,
            summary: "已修复".into(),
            error: None,
            finished_at: "2026-08-03T10:01:00Z".into(),
        };
        s.finish_task(&finished).unwrap();
        let row = s.get_task("t1").unwrap().unwrap();
        assert_eq!(row.status, "success");
        assert_eq!(row.total_tokens, 1100);
        // 聚合: prompt_tokens 500+300=800, completion 200+100=300
        assert_eq!(row.input_tokens, 800);
        assert_eq!(row.output_tokens, 300);
        assert_eq!(row.duration_secs, Some(45));
        assert!(row.finished_at.is_some());
    }

    #[test]
    fn finish_task_aggregates_ci_retries() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.insert_events(
            "t1",
            &vec![
                ExecutionEvent::CiStatus {
                    pipeline_id: 100,
                    status: CiStatus::Failed,
                    attempt: 1,
                },
                ExecutionEvent::CiStatus {
                    pipeline_id: 100,
                    status: CiStatus::Failed,
                    attempt: 2,
                },
                ExecutionEvent::CiStatus {
                    pipeline_id: 100,
                    status: CiStatus::Passed,
                    attempt: 3,
                },
            ],
        )
        .unwrap();
        let finished = TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 100,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:02:00Z".into(),
        };
        s.finish_task(&finished).unwrap();
        let row = s.get_task("t1").unwrap().unwrap();
        assert_eq!(row.ci_retries, 3);
    }

    // 注意: finish_task_writes_sop_deviations 测试已移至 Task 5 (依赖 sop_stats 方法)

    #[test]
    fn finish_task_unknown_returns_not_found() {
        let s = Storage::open_in_memory().unwrap();
        let finished = TaskFinishedEvent {
            task_id: "nope".into(),
            status: TaskStatus::Failed,
            duration_secs: 0,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: String::new(),
            error: None,
            finished_at: "2026-08-03T10:00:00Z".into(),
        };
        let err = s.finish_task(&finished).unwrap_err();
        assert!(matches!(err, crate::error::DashboardError::TaskNotFound(_)));
    }

    #[test]
    fn finish_task_twice_returns_conflict() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        let finished = TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:03:00Z".into(),
        };
        s.finish_task(&finished).unwrap();
        let err = s.finish_task(&finished).unwrap_err();
        assert!(matches!(err, crate::error::DashboardError::TaskConflict(_)));
    }

    #[test]
    fn task_is_finished_reports_correctly() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        assert!(!s.task_is_finished("t1").unwrap());
        let finished = TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:03:00Z".into(),
        };
        s.finish_task(&finished).unwrap();
        assert!(s.task_is_finished("t1").unwrap());
    }

    #[test]
    fn delete_task_removes_task_and_events() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.insert_events(
            "t1",
            &vec![ExecutionEvent::LlmCall {
                iteration: 1,
                prompt_tokens: 10,
                completion_tokens: 5,
                latency_ms: 100,
            }],
        )
        .unwrap();
        s.delete_task("t1").unwrap();
        assert!(!s.task_exists("t1").unwrap());
        assert!(s.list_events("t1").unwrap().is_empty());
    }

    #[test]
    fn list_tasks_pagination() {
        let s = Storage::open_in_memory().unwrap();
        for i in 0..15 {
            s.start_task(&sample_started(&format!("t{}", i))).unwrap();
        }
        let resp = s.list_tasks(1, 10, &TaskFilter::default()).unwrap();
        assert_eq!(resp.tasks.len(), 10);
        assert_eq!(resp.total, 15);
        assert_eq!(resp.page, 1);
        assert_eq!(resp.size, 10);
        let resp2 = s.list_tasks(2, 10, &TaskFilter::default()).unwrap();
        assert_eq!(resp2.tasks.len(), 5);
    }

    #[test]
    fn list_tasks_filter_by_status() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.start_task(&sample_started("t2")).unwrap();
        // t2 设为 success
        let f = TaskFinishedEvent {
            task_id: "t2".into(),
            status: TaskStatus::Success,
            duration_secs: 5,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:05:00Z".into(),
        };
        s.finish_task(&f).unwrap();
        let resp = s
            .list_tasks(1, 100, &TaskFilter { status: Some("running".into()), project: None })
            .unwrap();
        assert_eq!(resp.tasks.len(), 1);
        assert_eq!(resp.tasks[0].task_id, "t1");
    }

    #[test]
    fn list_tasks_filter_by_project() {
        let s = Storage::open_in_memory().unwrap();
        let mut a = sample_started("t1");
        a.project = "proj-a".into();
        let mut b = sample_started("t2");
        b.project = "proj-b".into();
        s.start_task(&a).unwrap();
        s.start_task(&b).unwrap();
        let resp = s
            .list_tasks(1, 100, &TaskFilter { status: None, project: Some("proj-a".into()) })
            .unwrap();
        assert_eq!(resp.tasks.len(), 1);
        assert_eq!(resp.tasks[0].task_id, "t1");
    }

    #[test]
    fn finish_task_writes_sop_deviations() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.insert_events(
            "t1",
            &vec![
                ExecutionEvent::SopStep {
                    step: "analyze".into(),
                    status: SopStepStatus::Completed,
                    note: None,
                },
                ExecutionEvent::SopStep {
                    step: "implement".into(),
                    status: SopStepStatus::Deviated,
                    note: Some("跳过单测".into()),
                },
            ],
        )
        .unwrap();
        let finished = TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:03:00Z".into(),
        };
        s.finish_task(&finished).unwrap();
        let devs = s.sop_stats().unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].step, "implement");
    }

    #[test]
    fn trends_returns_points() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        let f = TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 30,
            total_tokens: 1000,
            estimated_cost_usd: 0.05,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:05:00Z".into(),
        };
        s.finish_task(&f).unwrap();
        let data = s.trends(7).unwrap();
        assert_eq!(data.days, 7);
        assert!(!data.points.is_empty());
        let total: u64 = data.points.iter().map(|p| p.total).sum();
        assert_eq!(total, 1);
        let success: u64 = data.points.iter().map(|p| p.success).sum();
        assert_eq!(success, 1);
    }

    #[test]
    fn cost_breakdown_by_project() {
        let s = Storage::open_in_memory().unwrap();
        let mut a = sample_started("t1");
        a.project = "proj-a".into();
        let mut b = sample_started("t2");
        b.project = "proj-b".into();
        s.start_task(&a).unwrap();
        s.start_task(&b).unwrap();
        s.finish_task(&TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 500,
            estimated_cost_usd: 0.02,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:05:00Z".into(),
        })
        .unwrap();
        s.finish_task(&TaskFinishedEvent {
            task_id: "t2".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 300,
            estimated_cost_usd: 0.03,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:06:00Z".into(),
        })
        .unwrap();
        let buckets = s.cost_breakdown("project").unwrap();
        assert_eq!(buckets.len(), 2);
        let total_cost: f64 = buckets.iter().map(|b| b.total_cost_usd).sum();
        assert!((total_cost - 0.05).abs() < 1e-9);
    }

    #[test]
    fn cost_breakdown_invalid_group_returns_error() {
        let s = Storage::open_in_memory().unwrap();
        assert!(s.cost_breakdown("invalid").is_err());
    }

    #[test]
    fn ci_stats_counts_failures_and_heals() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.insert_events(
            "t1",
            &vec![ExecutionEvent::CiStatus {
                pipeline_id: 100,
                status: CiStatus::Failed,
                attempt: 2,
            }],
        )
        .unwrap();
        s.finish_task(&TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:05:00Z".into(),
        })
        .unwrap();
        s.start_task(&sample_started("t2")).unwrap();
        s.finish_task(&TaskFinishedEvent {
            task_id: "t2".into(),
            status: TaskStatus::CiFailed,
            duration_secs: 10,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "fail".into(),
            error: Some("CI 失败".into()),
            finished_at: "2026-08-03T10:06:00Z".into(),
        })
        .unwrap();
        let stats = s.ci_stats().unwrap();
        // t1: ci_retries=2 + status=success -> auto_healed
        // t2: status=ci_failed -> total_failed
        assert_eq!(stats.total_failed, 1);
        assert_eq!(stats.auto_healed, 1);
        assert_eq!(stats.failed_tasks.len(), 1);
    }

    #[test]
    fn sop_stats_returns_deviations() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.insert_events(
            "t1",
            &vec![ExecutionEvent::SopStep {
                step: "test".into(),
                status: SopStepStatus::Deviated,
                note: Some("跳过测试".into()),
            }],
        )
        .unwrap();
        s.finish_task(&TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:05:00Z".into(),
        })
        .unwrap();
        let devs = s.sop_stats().unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].step, "test");
        assert_eq!(devs[0].note.as_deref(), Some("跳过测试"));
    }
}
