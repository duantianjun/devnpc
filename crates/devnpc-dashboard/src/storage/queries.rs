//! Storage 结构与 CRUD/聚合/导入查询 (Task 3-6 填充)

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::Result;
use crate::storage::schema;

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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
