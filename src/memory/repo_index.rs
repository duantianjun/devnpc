//! 仓库索引 (P2 实现: 目录树构建、关键文件选择、摘要生成)

use crate::error::Result;
use super::context::{KeyFile, RepoTree};

/// 构建仓库目录树 (P2 实现)
pub fn build_repo_tree(_workspace: &std::path::Path) -> Result<RepoTree> {
    unimplemented!("P2 将实现")
}

/// 选择关键文件 (P2 实现)
pub fn select_key_files(_tree: &RepoTree) -> Vec<KeyFile> {
    unimplemented!("P2 将实现")
}
