//! SQLite schema 初始化
//!
//! 建表 + 索引。WAL 模式由 Storage::open 在连接时设置。

use rusqlite::Connection;

use crate::error::Result;

/// 全部建表 SQL (IF NOT EXISTS 保证幂等)
pub const SCHEMA_SQL: &str = r#"
-- 任务主表
CREATE TABLE IF NOT EXISTS tasks (
    task_id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    mr_iid INTEGER,
    pipeline_id INTEGER,
    task_description TEXT NOT NULL,
    task_kind TEXT NOT NULL,
    model TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    duration_secs INTEGER,
    total_tokens INTEGER DEFAULT 0,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    estimated_cost_usd REAL DEFAULT 0,
    mr_url TEXT,
    ci_url TEXT,
    summary TEXT,
    error TEXT,
    ci_retries INTEGER DEFAULT 0
);

-- 执行事件表
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (task_id) REFERENCES tasks(task_id)
);

-- SOP 偏离记录表
CREATE TABLE IF NOT EXISTS sop_deviations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    step TEXT NOT NULL,
    note TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (task_id) REFERENCES tasks(task_id)
);

-- 索引
CREATE INDEX IF NOT EXISTS idx_events_task_id ON events(task_id);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_project ON tasks(project);
CREATE INDEX IF NOT EXISTS idx_tasks_started_at ON tasks(started_at);
"#;

/// 执行全部建表与索引 (幂等,可重复调用)
pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_db_creates_tables() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        // 验证 tasks 表存在
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn init_db_creates_events_table() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn init_db_creates_sop_deviations_table() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sop_deviations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn init_db_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        // 再次执行不应报错
        init_db(&conn).unwrap();
    }

    #[test]
    fn init_db_creates_indexes() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        // 验证索引存在 (查 sqlite_master)
        let idx_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx_count, 4);
    }
}
