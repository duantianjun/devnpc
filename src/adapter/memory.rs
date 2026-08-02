//! 长期记忆系统: 跨会话积累项目知识和经验
//!
//! 轻量起步: 使用 SQLite 存储结构化记忆。
//! 包含: 任务记录、修复经验。

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::config::MemoryConfig;
use crate::error::Result;

/// 任务记录
#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub task_description: String,
    pub result_summary: String,
    pub modified_files: Vec<String>,
    pub duration_secs: u64,
    pub token_consumption: u64,
    pub success: bool,
    pub created_at: String,
}

/// 修复经验
#[derive(Debug, Clone)]
pub struct FixExperience {
    pub failure_type: String,
    pub error_message: String,
    pub root_cause: String,
    pub fix_method: String,
    pub success: bool,
    pub created_at: String,
}

/// 记忆存储器
pub struct MemoryStore {
    config: MemoryConfig,
    db_path: PathBuf,
    /// 持久的数据库连接，支持 :memory: 模式
    conn: Mutex<Option<Connection>>,
}

impl MemoryStore {
    pub fn new(config: MemoryConfig) -> Self {
        let db_path = PathBuf::from(&config.db_path);
        Self {
            config,
            db_path,
            conn: Mutex::new(None),
        }
    }

    /// 获取数据库连接（首次调用时创建，后续复用）
    fn connect(&self) -> std::result::Result<std::sync::MutexGuard<'_, Option<Connection>>, crate::error::DevnpcError> {
        let mut guard = self.conn.lock().map_err(|e| {
            crate::error::DevnpcError::Sqlite(format!("锁获取失败: {e}"))
        })?;
        if guard.is_none() {
            let conn = Connection::open(&self.db_path).map_err(|e| {
                crate::error::DevnpcError::Sqlite(format!("连接数据库失败: {e}"))
            })?;
            *guard = Some(conn);
        }
        Ok(guard)
    }

    /// 初始化数据库 (创建表)
    pub fn initialize(&self) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        tracing::info!(db_path = %self.db_path.display(), "初始化记忆存储");
        let guard = self.connect()?;
        let conn = guard.as_ref().ok_or_else(|| {
            crate::error::DevnpcError::Sqlite("数据库连接初始化失败".into())
        })?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS task_records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_description TEXT NOT NULL,
                result_summary TEXT NOT NULL,
                modified_files TEXT NOT NULL,
                duration_secs INTEGER NOT NULL,
                token_consumption INTEGER NOT NULL,
                success INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS fix_experiences (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                failure_type TEXT NOT NULL,
                error_message TEXT NOT NULL,
                root_cause TEXT NOT NULL,
                fix_method TEXT NOT NULL,
                success INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_task_records_created
                ON task_records(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_fix_experiences_type
                ON fix_experiences(failure_type);",
        )
        .map_err(|e| {
            crate::error::DevnpcError::Sqlite(format!("创建表失败: {e}"))
        })?;

        tracing::info!("记忆存储初始化完成");
        Ok(())
    }

    /// 保存任务记录
    pub fn save_task_record(&self, record: TaskRecord) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        let guard = self.connect()?;
        let conn = guard.as_ref().ok_or_else(|| {
            crate::error::DevnpcError::Sqlite("数据库连接初始化失败".into())
        })?;

        let files_json = serde_json::to_string(&record.modified_files)
            .unwrap_or_default();

        conn.execute(
            "INSERT INTO task_records
                (task_description, result_summary, modified_files,
                 duration_secs, token_consumption, success, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                record.task_description,
                record.result_summary,
                files_json,
                record.duration_secs as i64,
                record.token_consumption as i64,
                record.success as i64,
                record.created_at,
            ],
        )
        .map_err(|e| {
            crate::error::DevnpcError::Sqlite(format!("保存任务记录失败: {e}"))
        })?;

        tracing::debug!("任务记录已保存");
        Ok(())
    }

    /// 保存修复经验
    pub fn save_fix_experience(&self, exp: FixExperience) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        let guard = self.connect()?;
        let conn = guard.as_ref().ok_or_else(|| {
            crate::error::DevnpcError::Sqlite("数据库连接初始化失败".into())
        })?;

        conn.execute(
            "INSERT INTO fix_experiences
                (failure_type, error_message, root_cause, fix_method, success, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                exp.failure_type,
                exp.error_message,
                exp.root_cause,
                exp.fix_method,
                exp.success as i64,
                exp.created_at,
            ],
        )
        .map_err(|e| {
            crate::error::DevnpcError::Sqlite(format!("保存修复经验失败: {e}"))
        })?;

        tracing::debug!("修复经验已保存");
        Ok(())
    }

    /// 检索与当前任务相关的历史记忆
    ///
    /// 通过关键词匹配在 task_description 和 failure_type 中搜索。
    /// 返回格式化的记忆文本列表。
    pub fn retrieve_relevant(&self, task_description: &str) -> Result<Vec<String>> {
        if !self.config.enabled {
            return Ok(Vec::new());
        }
        let guard = self.connect()?;
        let conn = guard.as_ref().ok_or_else(|| {
            crate::error::DevnpcError::Sqlite("数据库连接初始化失败".into())
        })?;

        // 提取关键词 (取前 5 个非空词)
        let keywords: Vec<&str> = task_description
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .take(5)
            .collect();

        if keywords.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        // 搜索任务记录: 匹配任务描述中的关键词
        for kw in &keywords {
            let pattern = format!("%{}%", kw);
            let mut stmt = conn
                .prepare(
                    "SELECT task_description, result_summary, modified_files,
                            duration_secs, success, created_at
                     FROM task_records
                     WHERE task_description LIKE ?1
                     ORDER BY created_at DESC
                     LIMIT 3",
                )
                .map_err(|e| {
                    crate::error::DevnpcError::Sqlite(format!("查询准备失败: {e}"))
                })?;

            let rows = stmt
                .query_map(rusqlite::params![&pattern], |row| {
                    let desc: String = row.get(0)?;
                    let summary: String = row.get(1)?;
                    let files: String = row.get(2)?;
                    let duration: i64 = row.get(3)?;
                    let success: i64 = row.get(4)?;
                    let created: String = row.get(5)?;
                    Ok(format!(
                        "[任务] {created} | {desc}\n  结果: {summary}\n  文件: {files}\n  耗时: {duration}s | 状态: {}",
                        if success == 1 { "✓" } else { "✗" }
                    ))
                })
                .map_err(|e| {
                    crate::error::DevnpcError::Sqlite(format!("查询执行失败: {e}"))
                })?;

            for row in rows.flatten() {
                if !results.contains(&row) {
                    results.push(row);
                }
            }
        }

        // 搜索修复经验: 匹配失败类型或错误消息
        for kw in &keywords {
            let pattern = format!("%{}%", kw);
            let mut stmt = conn
                .prepare(
                    "SELECT failure_type, error_message, root_cause, fix_method, success, created_at
                     FROM fix_experiences
                     WHERE failure_type LIKE ?1 OR error_message LIKE ?1
                     ORDER BY created_at DESC
                     LIMIT 3",
                )
                .map_err(|e| {
                    crate::error::DevnpcError::Sqlite(format!("查询准备失败: {e}"))
                })?;

            let rows = stmt
                .query_map(rusqlite::params![&pattern], |row| {
                    let ftype: String = row.get(0)?;
                    let errmsg: String = row.get(1)?;
                    let cause: String = row.get(2)?;
                    let fix: String = row.get(3)?;
                    let success: i64 = row.get(4)?;
                    let created: String = row.get(5)?;
                    Ok(format!(
                        "[修复] {created} | {ftype}\n  错误: {errmsg}\n  根因: {cause}\n  修复: {fix}\n  状态: {}",
                        if success == 1 { "✓" } else { "✗" }
                    ))
                })
                .map_err(|e| {
                    crate::error::DevnpcError::Sqlite(format!("查询执行失败: {e}"))
                })?;

            for row in rows.flatten() {
                if !results.contains(&row) {
                    results.push(row);
                }
            }
        }

        // 限制返回数量
        results.truncate(10);
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_store() -> MemoryStore {
        let config = MemoryConfig {
            enabled: true,
            db_path: ":memory:".to_string(),
        };
        MemoryStore::new(config)
    }

    #[test]
    fn test_initialize_creates_tables() {
        let store = create_test_store();
        store.initialize().unwrap();

        // 验证表已创建: 插入一条记录
        let guard = store.connect().unwrap();
        let conn = guard.as_ref().unwrap();
        conn.execute(
            "INSERT INTO task_records
                (task_description, result_summary, modified_files,
                 duration_secs, token_consumption, success, created_at)
             VALUES ('test', 'ok', '[]', 10, 100, 1, '2026-01-01')",
            [],
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM task_records", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_save_and_retrieve_task_record() {
        let store = create_test_store();
        store.initialize().unwrap();

        let record = TaskRecord {
            task_description: "添加登录功能".to_string(),
            result_summary: "登录功能已实现".to_string(),
            modified_files: vec!["src/auth.rs".to_string()],
            duration_secs: 120,
            token_consumption: 5000,
            success: true,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        store.save_task_record(record).unwrap();

        let results = store.retrieve_relevant("登录功能").unwrap();
        assert!(!results.is_empty());
        assert!(results[0].contains("登录功能"));
    }

    #[test]
    fn test_save_fix_experience() {
        let store = create_test_store();
        store.initialize().unwrap();

        let exp = FixExperience {
            failure_type: "编译错误".to_string(),
            error_message: "E0277: cannot find value".to_string(),
            root_cause: "缺少变量声明".to_string(),
            fix_method: "添加 let x = ...".to_string(),
            success: true,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        store.save_fix_experience(exp).unwrap();

        let results = store.retrieve_relevant("编译错误 E0277").unwrap();
        assert!(!results.is_empty());
        assert!(results[0].contains("编译错误"));
    }

    #[test]
    fn test_disabled_store_does_nothing() {
        let config = MemoryConfig {
            enabled: false,
            db_path: ":memory:".to_string(),
        };
        let store = MemoryStore::new(config);
        // 不初始化也应该能安全调用
        assert!(store.retrieve_relevant("test").unwrap().is_empty());
        store
            .save_task_record(TaskRecord {
                task_description: "t".to_string(),
                result_summary: "s".to_string(),
                modified_files: vec![],
                duration_secs: 0,
                token_consumption: 0,
                success: true,
                created_at: String::new(),
            })
            .unwrap();
    }

    #[test]
    fn test_retrieve_relevant_returns_empty_for_short_keywords() {
        let store = create_test_store();
        store.initialize().unwrap();
        // 所有词长度 <= 2
        let results = store.retrieve_relevant("a b c").unwrap();
        assert!(results.is_empty());
    }
}