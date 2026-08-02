//! 仓库元数据/文件操作高层 helper
//!
//! 在 `GitlabApi` trait 的 `get_file` (raw 内容) / `list_tree` 之上,提供:
//! - 便捷读取文件内容 (默认 ref=main)
//! - 列出根目录树
//! - 从树条目中过滤文件/目录
//! - 批量读取多文件内容

use crate::error::Result;
use crate::gitlab_api::{GitlabApi, RepoTreeEntry};

/// 默认分支名 (未指定 ref 时使用)
pub const DEFAULT_REF: &str = "main";

/// 读取仓库文件内容 (使用默认 ref=main)
pub async fn read_file(
    gitlab: &dyn GitlabApi,
    project_id: u64,
    file_path: &str,
) -> Result<String> {
    gitlab.get_file(project_id, file_path, DEFAULT_REF).await
}

/// 读取仓库文件内容 (指定 ref)
pub async fn read_file_at(
    gitlab: &dyn GitlabApi,
    project_id: u64,
    file_path: &str,
    ref_: &str,
) -> Result<String> {
    gitlab.get_file(project_id, file_path, ref_).await
}

/// 列出根目录树 (使用默认 ref=main)
pub async fn list_root_tree(gitlab: &dyn GitlabApi, project_id: u64) -> Result<Vec<RepoTreeEntry>> {
    gitlab.list_tree(project_id, "", DEFAULT_REF).await
}

/// 列出指定路径下的目录树 (使用默认 ref=main)
pub async fn list_path(gitlab: &dyn GitlabApi, project_id: u64, path: &str) -> Result<Vec<RepoTreeEntry>> {
    gitlab.list_tree(project_id, path, DEFAULT_REF).await
}

/// 从树条目中过滤出文件 (type=blob)
pub fn filter_files(entries: &[RepoTreeEntry]) -> Vec<&RepoTreeEntry> {
    entries.iter().filter(|e| e.type_ == "blob").collect()
}

/// 从树条目中过滤出子目录 (type=tree)
pub fn filter_dirs(entries: &[RepoTreeEntry]) -> Vec<&RepoTreeEntry> {
    entries.iter().filter(|e| e.type_ == "tree").collect()
}

/// 按文件名查找树条目 (精确匹配,大小写敏感)
pub fn find_by_name<'a>(entries: &'a [RepoTreeEntry], name: &str) -> Option<&'a RepoTreeEntry> {
    entries.iter().find(|e| e.name == name)
}

/// 批量读取多个文件内容 (顺序拉取,任一失败立即返回错误)
pub async fn read_files(
    gitlab: &dyn GitlabApi,
    project_id: u64,
    paths: &[&str],
    ref_: &str,
) -> Result<Vec<String>> {
    let mut result = Vec::with_capacity(paths.len());
    for path in paths {
        result.push(gitlab.get_file(project_id, path, ref_).await?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab_api::{
        CreateMrReq, Issue, Job, MergeRequest, Note, Pipeline,
    };
    use async_trait::async_trait;
    use std::sync::Mutex;

    fn make_entry(name: &str, type_: &str, path: &str) -> RepoTreeEntry {
        RepoTreeEntry {
            id: format!("id-{name}"),
            name: name.into(),
            type_: type_.into(),
            path: path.into(),
            mode: "100644".into(),
        }
    }

    /// 可注入文件内容与树条目的 Mock
    struct RepoMock {
        files: std::collections::HashMap<String, String>,
        tree: Vec<RepoTreeEntry>,
        file_calls: Mutex<u32>,
    }

    #[async_trait]
    impl GitlabApi for RepoMock {
        async fn get_issue(&self, _p: u64, _i: u64) -> Result<Issue> {
            unreachable!()
        }
        async fn get_mr(&self, _p: u64, _i: u64) -> Result<MergeRequest> {
            unreachable!()
        }
        async fn create_mr(&self, _p: u64, _r: CreateMrReq) -> Result<MergeRequest> {
            unreachable!()
        }
        async fn get_pipelines(&self, _p: u64) -> Result<Vec<Pipeline>> {
            Ok(vec![])
        }
        async fn get_issue_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
            Ok(vec![])
        }
        async fn get_mr_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
            Ok(vec![])
        }
        async fn create_mr_note(&self, _p: u64, _i: u64, _b: &str) -> Result<Note> {
            unreachable!()
        }
        async fn create_issue_note(&self, _p: u64, _i: u64, _b: &str) -> Result<Note> {
            unreachable!()
        }
        async fn get_related_mrs(&self, _p: u64, _i: u64) -> Result<Vec<MergeRequest>> {
            Ok(vec![])
        }
        async fn get_recent_pipelines(&self, _p: u64, _c: usize) -> Result<Vec<Pipeline>> {
            Ok(vec![])
        }
        async fn update_mr(&self, _p: u64, _i: u64, _t: &str, _d: bool) -> Result<MergeRequest> {
            unreachable!()
        }
        async fn get_pipeline_jobs(&self, _p: u64, _pi: u64) -> Result<Vec<Job>> {
            Ok(vec![])
        }
        async fn get_job_log(&self, _p: u64, _j: u64) -> Result<String> {
            Ok(String::new())
        }
        async fn get_pipeline(&self, _p: u64, _pi: u64) -> Result<Pipeline> {
            Ok(Pipeline {
                id: 1,
                status: "success".into(),
                ref_: None,
                sha: None,
                web_url: String::new(),
            })
        }
        async fn get_file(&self, _p: u64, fp: &str, _r: &str) -> Result<String> {
            *self.file_calls.lock().unwrap() += 1;
            Ok(self
                .files
                .get(fp)
                .cloned()
                .unwrap_or_else(|| format!("[not found: {fp}]")))
        }
        async fn list_tree(&self, _p: u64, _path: &str, _r: &str) -> Result<Vec<RepoTreeEntry>> {
            Ok(self.tree.clone())
        }
    }

    #[tokio::test]
    async fn read_file_uses_main_ref() {
        let mut files = std::collections::HashMap::new();
        files.insert("README.md".into(), "# Test\n".into());
        let mock = RepoMock {
            files,
            tree: vec![],
            file_calls: Mutex::new(0),
        };
        let content = read_file(&mock, 1, "README.md").await.unwrap();
        assert_eq!(content, "# Test\n");
    }

    #[tokio::test]
    async fn read_file_at_uses_provided_ref() {
        let mut files = std::collections::HashMap::new();
        files.insert("Cargo.toml".into(), "[package]\n".into());
        let mock = RepoMock {
            files,
            tree: vec![],
            file_calls: Mutex::new(0),
        };
        let content = read_file_at(&mock, 1, "Cargo.toml", "dev").await.unwrap();
        assert_eq!(content, "[package]\n");
    }

    #[tokio::test]
    async fn list_root_tree_returns_entries() {
        let mock = RepoMock {
            files: std::collections::HashMap::new(),
            tree: vec![
                make_entry("README.md", "blob", "README.md"),
                make_entry("src", "tree", "src"),
            ],
            file_calls: Mutex::new(0),
        };
        let entries = list_root_tree(&mock, 1).await.unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn read_files_collects_multiple() {
        let mut files = std::collections::HashMap::new();
        files.insert("a.rs".into(), "fn a() {}".into());
        files.insert("b.rs".into(), "fn b() {}".into());
        let mock = RepoMock {
            files,
            tree: vec![],
            file_calls: Mutex::new(0),
        };
        let contents = read_files(&mock, 1, &["a.rs", "b.rs"], "main").await.unwrap();
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0], "fn a() {}");
        assert_eq!(contents[1], "fn b() {}");
        assert_eq!(*mock.file_calls.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn read_files_propagates_error() {
        // Mock 总是返回 NotFound
        struct ErrMock;
        #[async_trait]
        impl GitlabApi for ErrMock {
            async fn get_issue(&self, _p: u64, _i: u64) -> Result<Issue> {
                unreachable!()
            }
            async fn get_mr(&self, _p: u64, _i: u64) -> Result<MergeRequest> {
                unreachable!()
            }
            async fn create_mr(&self, _p: u64, _r: CreateMrReq) -> Result<MergeRequest> {
                unreachable!()
            }
            async fn get_pipelines(&self, _p: u64) -> Result<Vec<Pipeline>> {
                Ok(vec![])
            }
            async fn get_issue_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
                Ok(vec![])
            }
            async fn get_mr_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
                Ok(vec![])
            }
            async fn create_mr_note(&self, _p: u64, _i: u64, _b: &str) -> Result<Note> {
                unreachable!()
            }
            async fn create_issue_note(&self, _p: u64, _i: u64, _b: &str) -> Result<Note> {
                unreachable!()
            }
            async fn get_related_mrs(&self, _p: u64, _i: u64) -> Result<Vec<MergeRequest>> {
                Ok(vec![])
            }
            async fn get_recent_pipelines(&self, _p: u64, _c: usize) -> Result<Vec<Pipeline>> {
                Ok(vec![])
            }
            async fn update_mr(&self, _p: u64, _i: u64, _t: &str, _d: bool) -> Result<MergeRequest> {
                unreachable!()
            }
            async fn get_pipeline_jobs(&self, _p: u64, _pi: u64) -> Result<Vec<Job>> {
                Ok(vec![])
            }
            async fn get_job_log(&self, _p: u64, _j: u64) -> Result<String> {
                Ok(String::new())
            }
            async fn get_pipeline(&self, _p: u64, _pi: u64) -> Result<Pipeline> {
                Ok(Pipeline {
                    id: 1,
                    status: "success".into(),
                    ref_: None,
                    sha: None,
                    web_url: String::new(),
                })
            }
            async fn get_file(&self, _p: u64, _fp: &str, _r: &str) -> Result<String> {
                Err(crate::error::DevnpcError::GitlabNotFound {
                    resource: "file".into(),
                })
            }
            async fn list_tree(&self, _p: u64, _path: &str, _r: &str) -> Result<Vec<RepoTreeEntry>> {
                Ok(vec![])
            }
        }
        let result = read_files(&ErrMock, 1, &["missing.rs"], "main").await;
        assert!(result.is_err());
    }

    #[test]
    fn filter_files_returns_only_blobs() {
        let entries = vec![
            make_entry("README.md", "blob", "README.md"),
            make_entry("src", "tree", "src"),
            make_entry("Cargo.toml", "blob", "Cargo.toml"),
        ];
        let files = filter_files(&entries);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, "README.md");
        assert_eq!(files[1].name, "Cargo.toml");
    }

    #[test]
    fn filter_dirs_returns_only_trees() {
        let entries = vec![
            make_entry("README.md", "blob", "README.md"),
            make_entry("src", "tree", "src"),
            make_entry("tests", "tree", "tests"),
        ];
        let dirs = filter_dirs(&entries);
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[0].name, "src");
        assert_eq!(dirs[1].name, "tests");
    }

    #[test]
    fn find_by_name_returns_matching_entry() {
        let entries = vec![
            make_entry("README.md", "blob", "README.md"),
            make_entry("Cargo.toml", "blob", "Cargo.toml"),
        ];
        let found = find_by_name(&entries, "Cargo.toml").unwrap();
        assert_eq!(found.path, "Cargo.toml");
        assert!(find_by_name(&entries, "missing").is_none());
    }

    #[test]
    fn default_ref_is_main() {
        assert_eq!(DEFAULT_REF, "main");
    }
}
