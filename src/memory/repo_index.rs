//! 仓库索引 (P2 实现: 目录树构建、关键文件选择、摘要生成)

use std::path::Path;

use crate::config::SummaryConfig;
use crate::error::Result;
use crate::git::ops::GitOps;
use crate::memory::context::{KeyFile, RepoTree, TreeEntry, TreeKind};

/// 构建仓库目录树 (顶层 + 二级)
///
/// 用 `git ls-tree HEAD` 获取顶层,对顶层目录再 ls-tree 获取二级。
/// 两层结构,token 可控,够 Agent 理解项目布局。
pub fn build_repo_tree(workspace: &Path) -> Result<RepoTree> {
    let ops = GitOps::new(workspace);
    let mut entries = Vec::new();

    // 顶层
    let top_out = ops.ls_tree_head()?;
    for entry in parse_ls_tree(&top_out) {
        entries.push(entry.clone());
        // 对顶层目录,深入二级
        if entry.kind == TreeKind::Dir {
            let sub_out = ops.ls_tree_subdir(&entry.path)?;
            for sub in parse_ls_tree(&sub_out) {
                // git ls-tree HEAD <subdir> 返回的 path 已是相对仓库根的完整路径
                // (如 "src/main.rs"),无需拼接
                entries.push(sub);
            }
        }
    }

    Ok(RepoTree { entries })
}

/// 解析 `git ls-tree` 输出
/// 行格式: `<mode> <type> <hash>\t<path>`
fn parse_ls_tree(output: &str) -> Vec<TreeEntry> {
    output
        .lines()
        .filter_map(|line| {
            // 找到 tab 分隔 path
            let tab_idx = line.find('\t')?;
            let meta = &line[..tab_idx];
            let path = &line[tab_idx + 1..];
            let parts: Vec<&str> = meta.split_whitespace().collect();
            if parts.len() < 2 {
                return None;
            }
            let kind = match parts[1] {
                "tree" => TreeKind::Dir,
                "blob" => TreeKind::File,
                _ => return None,
            };
            Some(TreeEntry {
                path: path.to_string(),
                kind,
                size: None,
            })
        })
        .collect()
}

/// 选择关键文件并生成摘要
///
/// 摘要规则 (降 token):
/// - Cargo.toml → 保留 [dependencies] 段 (若存在),否则前 readme_lines 行
/// - README.md / .devnpc.md → 前 readme_lines 行
/// - src/main.rs / src/lib.rs → 前 main_rs_lines 行
/// - 其他 → 前 other_lines 行
pub fn select_key_files(tree: &RepoTree, workspace: &Path, config: &SummaryConfig) -> Vec<KeyFile> {
    let mut key_files = Vec::new();
    for entry in &tree.entries {
        if entry.kind != TreeKind::File {
            continue;
        }
        if !config.key_file_patterns.contains(&entry.path) {
            continue;
        }
        let full_path = workspace.join(&entry.path);
        let Ok(content) = std::fs::read_to_string(&full_path) else {
            continue;
        };
        let summary = summarize(&entry.path, &content, config);
        key_files.push(KeyFile {
            path: entry.path.clone(),
            summary,
        });
    }
    key_files
}

/// 按文件类型生成摘要
fn summarize(path: &str, content: &str, config: &SummaryConfig) -> String {
    match path {
        "Cargo.toml" => summarize_cargo_toml(content, config),
        "README.md" | ".devnpc.md" => take_first_n_lines(content, config.readme_lines),
        "src/main.rs" | "src/lib.rs" => take_first_n_lines(content, config.main_rs_lines),
        _ => take_first_n_lines(content, config.other_lines),
    }
}

/// Cargo.toml 摘要: 保留 [dependencies] 段;若无则返回前 readme_lines 行
fn summarize_cargo_toml(content: &str, config: &SummaryConfig) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    let mut in_deps = false;
    for line in &lines {
        if line.starts_with('[') {
            in_deps = line.trim() == "[dependencies]";
        }
        if in_deps {
            result.push(*line);
        }
    }
    if result.is_empty() {
        // 无 [dependencies] 段,返回前 readme_lines 行
        take_first_n_lines(content, config.readme_lines)
    } else {
        result.join("\n")
    }
}

/// 取前 N 行
fn take_first_n_lines(content: &str, n: usize) -> String {
    content.lines().take(n).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SummaryConfig;
    use crate::git::ops::GitOps;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn default_summary_config() -> SummaryConfig {
        SummaryConfig::default()
    }

    fn setup_temp_repo() -> (TempDir, GitOps) {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        fs::create_dir_all(&repo_path).unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "t@t.com"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "T"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        fs::write(repo_path.join("Cargo.toml"), "[package]\nname=\"t\"\n").unwrap();
        fs::write(repo_path.join("README.md"), "# T\n").unwrap();
        fs::create_dir_all(repo_path.join("src")).unwrap();
        fs::write(repo_path.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::create_dir_all(repo_path.join("src/handler")).unwrap();
        fs::write(repo_path.join("src/handler/login.rs"), "pub fn login() {}\n").unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
        let ops = GitOps::new(&repo_path);
        (dir, ops)
    }

    #[test]
    fn build_repo_tree_contains_top_level_files_and_dirs() {
        let (_dir, ops) = setup_temp_repo();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let paths: Vec<&str> = tree.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"README.md"));
        assert!(paths.contains(&"src"));
        // src 是目录
        let src_entry = tree.entries.iter().find(|e| e.path == "src").unwrap();
        assert_eq!(src_entry.kind, crate::memory::context::TreeKind::Dir);
    }

    #[test]
    fn build_repo_tree_includes_second_level_entries() {
        let (_dir, ops) = setup_temp_repo();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        // 二级: src/main.rs, src/handler
        let paths: Vec<&str> = tree.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&"src/handler"));
    }

    #[test]
    fn build_repo_tree_marks_files_as_file_kind() {
        let (_dir, ops) = setup_temp_repo();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let cargo = tree
            .entries
            .iter()
            .find(|e| e.path == "Cargo.toml")
            .unwrap();
        assert_eq!(cargo.kind, crate::memory::context::TreeKind::File);
    }

    #[test]
    fn select_key_files_picks_cargo_toml_and_readme() {
        let (_dir, ops) = setup_temp_repo();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let key_files = select_key_files(&tree, &ops.workspace, &default_summary_config());
        let paths: Vec<&str> = key_files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"README.md"));
    }

    #[test]
    fn select_key_files_cargo_toml_summary_includes_package_name() {
        let (_dir, ops) = setup_temp_repo();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let key_files = select_key_files(&tree, &ops.workspace, &default_summary_config());
        let cargo = key_files
            .iter()
            .find(|f| f.path == "Cargo.toml")
            .unwrap();
        // 摘要应包含 [package] 段 (无 [dependencies] 也至少有 package)
        assert!(cargo.summary.contains("[package]") || cargo.summary.contains("name"));
    }

    #[test]
    fn select_key_files_readme_summary_capped_at_30_lines() {
        let (_dir, ops) = setup_temp_repo();
        // 写一个长 README
        let long_readme = (1..=50)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(ops.workspace.join("README.md"), long_readme).unwrap();
        // select_key_files 读工作区文件,不需重新 commit
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let key_files = select_key_files(&tree, &ops.workspace, &default_summary_config());
        let readme = key_files
            .iter()
            .find(|f| f.path == "README.md")
            .unwrap();
        let line_count = readme.summary.lines().count();
        assert!(line_count <= 30, "README 摘要应 ≤30 行,实际 {line_count}");
    }

    #[test]
    fn select_key_files_picks_src_main_rs() {
        let (_dir, ops) = setup_temp_repo();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let key_files = select_key_files(&tree, &ops.workspace, &default_summary_config());
        let paths: Vec<&str> = key_files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"));
    }

    #[test]
    fn select_key_files_ignores_non_key_files() {
        let (_dir, ops) = setup_temp_repo();
        // 加一个非关键文件
        std::fs::write(ops.workspace.join("random.txt"), "data").unwrap();
        let tree = build_repo_tree(&ops.workspace).unwrap();
        let key_files = select_key_files(&tree, &ops.workspace, &default_summary_config());
        let paths: Vec<&str> = key_files.iter().map(|f| f.path.as_str()).collect();
        assert!(!paths.contains(&"random.txt"));
    }
}
