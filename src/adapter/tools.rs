//! 工具适配: 将 devnpc 业务工具包装为 adk-rust FunctionTool
//!
//! 保留 src/tools/ 下各工具的实现逻辑,通过本模块适配为框架可用的 FunctionTool。
//! Handler 签名: Fn(Arc<dyn ToolContext>, Value) -> Future<Output = Result<Value>>

use std::path::PathBuf;
use std::sync::Arc;

use adk_rust::tool::FunctionTool;
use adk_rust::Tool;
use serde_json::Value;

use crate::config::Config;
use crate::git::ops::GitOps;
use crate::gitlab_api::GitlabApi;
use crate::adapter::file_io::FileIo;

/// 创建所有业务工具的 FunctionTool 包装
///
/// `gitlab` 和 `project_id` 为可选,仅在需要 create_mr_note 工具时传入。
/// `mcp_tools` 为可选,来自 MCP Gateway 收集的远程工具。
pub fn create_all_tools(
    config: &Config,
    gitlab: Option<Arc<dyn GitlabApi>>,
    project_id: Option<u64>,
    mcp_tools: Vec<Arc<dyn Tool>>,
) -> Vec<Arc<dyn Tool>> {
    let workspace = std::env::current_dir().expect("获取工作目录失败");
    let file_io = FileIo::new(&workspace);
    let git_ops = GitOps::new(&workspace);

    let mut tools: Vec<Arc<dyn Tool>> = vec![
        // 文件工具
        Arc::new(create_read_file_tool(file_io.clone(), &config.read_file)),
        Arc::new(create_write_file_tool(file_io.clone())),
        Arc::new(create_list_files_tool(file_io.clone())),
        // Shell 工具
        Arc::new(create_run_command_tool(workspace.clone(), &config.command)),
        // Git 工具
        Arc::new(create_git_diff_tool(workspace.clone())),
        Arc::new(create_git_commit_tool(git_ops)),
        // AFT 代码感知工具
        Arc::new(create_aft_outline_tool(file_io.clone())),
        Arc::new(create_aft_view_symbol_tool(file_io.clone())),
        Arc::new(create_aft_edit_symbol_tool(file_io.clone())),
        Arc::new(create_aft_search_symbols_tool(file_io.clone())),
        Arc::new(create_aft_ast_replace_tool(file_io)),
    ];

    // 可选: GitLab 工具 (需要外部客户端)
    if let (Some(gitlab), Some(pid)) = (gitlab, project_id) {
        tools.push(Arc::new(create_mr_note_tool(gitlab, pid)));
    }

    // 合并 MCP 工具
    tools.extend(mcp_tools);

    tools
}

/// read_file: 读取 workspace 内文件 (限前 N 行)
fn create_read_file_tool(file_io: FileIo, read_config: &crate::config::ReadFileConfig) -> FunctionTool {
    let max_lines = read_config.max_lines;
    FunctionTool::new(
        "read_file",
        "读取 workspace 内文件全文 (限前 {max_lines} 行)。path 相对 workspace 根。",
        move |_ctx: Arc<dyn adk_rust::tool::ToolContext>, args: Value| {
            let file_io = file_io.clone();
            let max_lines = max_lines;
            Box::pin(async move {
                let path = args["path"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 path 参数")
                })?;
                let full = file_io.validate_path(path).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("路径验证失败: {e}"))
                })?;
                let content = std::fs::read_to_string(&full).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("读取失败: {e}"))
                })?;
                let truncated: String = content
                    .lines()
                    .take(max_lines)
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(serde_json::json!({ "content": truncated }))
            })
        },
    )
}

/// write_file: 写入 workspace 内文件 (全量覆盖)
fn create_write_file_tool(file_io: FileIo) -> FunctionTool {
    FunctionTool::new(
        "write_file",
        "写入 workspace 内文件 (全量覆盖)。path 相对 workspace 根, content 为完整文件内容。",
        move |_ctx: Arc<dyn adk_rust::tool::ToolContext>, args: Value| {
            let file_io = file_io.clone();
            Box::pin(async move {
                let path = args["path"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 path 参数")
                })?;
                let content = args["content"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 content 参数")
                })?;
                let full = file_io.validate_path(path).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("路径验证失败: {e}"))
                })?;
                if let Some(parent) = full.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("创建目录失败: {e}"))
                    })?;
                }
                std::fs::write(&full, content).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("写入失败: {e}"))
                })?;
                Ok(serde_json::json!({ "status": "ok", "path": path }))
            })
        },
    )
}

/// list_files: 列出 workspace 内目录的条目
fn create_list_files_tool(file_io: FileIo) -> FunctionTool {
    FunctionTool::new(
        "list_files",
        "列出 workspace 内指定目录的条目 (文件/子目录名)。dir 相对 workspace 根, 默认当前目录。",
        move |_ctx: Arc<dyn adk_rust::tool::ToolContext>, args: Value| {
            let file_io = file_io.clone();
            Box::pin(async move {
                let dir = args["dir"].as_str().unwrap_or("");
                let full = file_io.validate_path(dir).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("路径验证失败: {e}"))
                })?;
                if !full.is_dir() {
                    return Ok(serde_json::json!({ "error": format!("不是目录: {dir}") }));
                }
                let mut entries: Vec<String> = std::fs::read_dir(&full)
                    .map_err(|e| {
                        adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("读取目录失败: {e}"))
                    })?
                    .filter_map(|e| e.ok())
                    .map(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        if e.path().is_dir() {
                            format!("{name}/")
                        } else {
                            name
                        }
                    })
                    .collect();
                entries.sort();
                Ok(serde_json::json!({ "entries": entries }))
            })
        },
    )
}

/// run_command: 在 workspace 内执行白名单命令
fn create_run_command_tool(
    workspace: std::path::PathBuf,
    cmd_config: &crate::config::CommandConfig,
) -> FunctionTool {
    let allowlist = cmd_config.allowlist.clone();
    let denylist = cmd_config.denylist.clone();
    let default_timeout = std::time::Duration::from_secs(cmd_config.default_timeout_secs);

    FunctionTool::new(
        "run_command",
        "在 workspace 内执行白名单命令 (cargo/mvn/gradle/npm/python/go 等)。参数: cmd, args, timeout_secs。",
        move |_ctx: Arc<dyn adk_rust::tool::ToolContext>, args: Value| {
            let workspace = workspace.clone();
            let allowlist = allowlist.clone();
            let denylist = denylist.clone();
            let default_timeout = default_timeout;
            Box::pin(async move {
                let cmd = args["cmd"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 cmd 参数")
                })?;

                // 黑名单优先
                if denylist.iter().any(|d| d == cmd) {
                    return Ok(serde_json::json!({
                        "error": format!("命令 {cmd} 在黑名单中"),
                        "success": false
                    }));
                }
                // 白名单检查
                if !allowlist.iter().any(|a| a == cmd) {
                    return Ok(serde_json::json!({
                        "error": format!("命令 {cmd} 不在白名单中 (允许: {})", allowlist.join(", ")),
                        "success": false
                    }));
                }

                let cmd_args: Vec<String> = args["args"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                let timeout = args["timeout_secs"]
                    .as_u64()
                    .map(std::time::Duration::from_secs)
                    .unwrap_or(default_timeout);

                let mut process = tokio::process::Command::new(cmd);
                process
                    .args(&cmd_args)
                    .current_dir(&workspace)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());

                let child = process.spawn().map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("启动命令失败: {e}"))
                })?;

                match tokio::time::timeout(timeout, child.wait_with_output()).await {
                    Ok(Ok(output)) => {
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        let success = output.status.success();
                        let combined = if stderr.is_empty() {
                            stdout
                        } else {
                            format!("{stdout}\n[stderr]\n{stderr}")
                        };
                        Ok(serde_json::json!({
                            "output": combined,
                            "success": success,
                            "exit_code": output.status.code()
                        }))
                    }
                    Ok(Err(e)) => Ok(serde_json::json!({
                        "error": format!("等待命令失败: {e}"),
                        "success": false
                    })),
                    Err(_) => Ok(serde_json::json!({
                        "error": format!("命令超时 ({timeout:?})"),
                        "success": false
                    })),
                }
            })
        },
    )
}

// ============================================================
// Git 工具
// ============================================================

/// git_diff: 查看工作区相对 HEAD 的未提交改动
fn create_git_diff_tool(workspace: PathBuf) -> FunctionTool {
    FunctionTool::new(
        "git_diff",
        "查看当前工作区相对 HEAD 的未提交改动 (git diff HEAD)。无改动返回空字符串。",
        move |_ctx: Arc<dyn adk_rust::tool::ToolContext>, args: Value| {
            let workspace = workspace.clone();
            Box::pin(async move {
                let _ = args; // 无参数
                let output = std::process::Command::new("git")
                    .args(["diff", "HEAD"])
                    .current_dir(&workspace)
                    .output()
                    .map_err(|e| {
                        adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("执行 git diff 失败: {e}"))
                    })?;
                let success = output.status.success();
                let diff = String::from_utf8_lossy(&output.stdout).to_string();
                Ok(serde_json::json!({
                    "output": diff,
                    "success": success,
                }))
            })
        },
    )
}

/// git_commit: 提交当前所有改动 (git add -A + git commit)
fn create_git_commit_tool(git_ops: GitOps) -> FunctionTool {
    FunctionTool::new(
        "git_commit",
        "提交当前所有改动 (git add -A + git commit)。参数: message (commit message)。",
        move |_ctx: Arc<dyn adk_rust::tool::ToolContext>, args: Value| {
            let git_ops = git_ops.clone();
            Box::pin(async move {
                let message = args["message"].as_str().unwrap_or("");
                if message.is_empty() {
                    return Ok(serde_json::json!({
                        "error": "message 不能为空",
                        "success": false
                    }));
                }
                match git_ops.commit(message).await {
                    Ok(_) => Ok(serde_json::json!({
                        "output": format!("已提交: {message}"),
                        "success": true
                    })),
                    Err(e) => Ok(serde_json::json!({
                        "error": format!("提交失败: {e}"),
                        "success": false
                    })),
                }
            })
        },
    )
}

// ============================================================
// GitLab 工具
// ============================================================

/// create_mr_note: 在指定 MR 发表评论
fn create_mr_note_tool(gitlab: Arc<dyn GitlabApi>, project_id: u64) -> FunctionTool {
    FunctionTool::new(
        "create_mr_note",
        "在指定 MR 发表评论。参数: mr_iid (MR iid), body (评论内容)。",
        move |_ctx: Arc<dyn adk_rust::tool::ToolContext>, args: Value| {
            let gitlab = gitlab.clone();
            let project_id = project_id;
            Box::pin(async move {
                let mr_iid = args["mr_iid"].as_u64().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 mr_iid 参数")
                })?;
                let body = args["body"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 body 参数")
                })?;
                match gitlab.create_mr_note(project_id, mr_iid, body).await {
                    Ok(note) => Ok(serde_json::json!({
                        "output": format!("已评论 MR !{} (note_id={})", mr_iid, note.id),
                        "success": true
                    })),
                    Err(e) => Ok(serde_json::json!({
                        "error": format!("评论失败: {e}"),
                        "success": false
                    })),
                }
            })
        },
    )
}

// ============================================================
// AFT 代码感知工具 (基于 tree-sitter, 支持 Rust + Java + Python + JS/TS + Go + C/C++)
// ============================================================

/// 支持的语言
#[derive(Clone, Copy, PartialEq, Debug)]
enum Language {
    Rust,
    Java,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
    C,
    Cpp,
}

/// 根据文件扩展名检测语言
fn detect_language(path: &std::path::Path) -> Option<Language> {
    match path.extension()?.to_str()? {
        "rs" => Some(Language::Rust),
        "java" => Some(Language::Java),
        "py" => Some(Language::Python),
        "js" | "jsx" => Some(Language::JavaScript),
        "ts" => Some(Language::TypeScript),
        "tsx" => Some(Language::Tsx),
        "go" => Some(Language::Go),
        "c" | "h" => Some(Language::C),
        "cpp" | "hpp" | "cc" | "cxx" => Some(Language::Cpp),
        _ => None,
    }
}

/// 获取 tree-sitter 语言的 Language 引用
fn get_language(lang: Language) -> tree_sitter::Language {
    match lang {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Java => tree_sitter_java::LANGUAGE.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::C => tree_sitter_c::LANGUAGE.into(),
        Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
    }
}

/// 返回对应语言的关注 AST 节点类型 (顶层声明)
fn interesting_kinds(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Rust => &[
            "function_item", "struct_item", "enum_item", "trait_item",
            "impl_item", "mod_item", "const_item", "static_item",
            "type_item", "macro_definition", "union_item", "foreign_mod_item",
        ],
        Language::Java => &[
            "class_declaration", "interface_declaration", "enum_declaration",
            "record_declaration", "method_declaration", "constructor_declaration",
            "field_declaration", "annotation_type_declaration",
        ],
        Language::Python => &[
            "function_definition", "class_definition",
        ],
        Language::JavaScript | Language::TypeScript | Language::Tsx => &[
            "function_declaration", "generator_function_declaration",
            "class_declaration", "method_definition",
            "interface_declaration", "type_alias_declaration", "enum_declaration",
        ],
        Language::Go => &[
            "function_declaration", "method_declaration", "type_declaration",
            "type_spec",
        ],
        Language::C => &[
            "function_definition", "struct_specifier", "union_specifier",
            "enum_specifier", "type_definition",
        ],
        Language::Cpp => &[
            "function_definition", "class_specifier", "struct_specifier",
            "union_specifier", "enum_specifier", "type_definition",
        ],
    }
}

/// 解析源码的 tree-sitter 辅助函数 (支持多语言,使用线程局部缓存)
fn parse_source(source: &str, lang: Language) -> tree_sitter::Tree {
    use std::cell::RefCell;
    thread_local! {
        static RUST_PARSER: RefCell<Option<tree_sitter::Parser>> = const { RefCell::new(None) };
        static JAVA_PARSER: RefCell<Option<tree_sitter::Parser>> = const { RefCell::new(None) };
        static PYTHON_PARSER: RefCell<Option<tree_sitter::Parser>> = const { RefCell::new(None) };
        static JS_PARSER: RefCell<Option<tree_sitter::Parser>> = const { RefCell::new(None) };
        static TS_PARSER: RefCell<Option<tree_sitter::Parser>> = const { RefCell::new(None) };
        static TSX_PARSER: RefCell<Option<tree_sitter::Parser>> = const { RefCell::new(None) };
        static GO_PARSER: RefCell<Option<tree_sitter::Parser>> = const { RefCell::new(None) };
        static C_PARSER: RefCell<Option<tree_sitter::Parser>> = const { RefCell::new(None) };
        static CPP_PARSER: RefCell<Option<tree_sitter::Parser>> = const { RefCell::new(None) };
    }
    let parser_cell = match lang {
        Language::Rust => &RUST_PARSER,
        Language::Java => &JAVA_PARSER,
        Language::Python => &PYTHON_PARSER,
        Language::JavaScript => &JS_PARSER,
        Language::TypeScript => &TS_PARSER,
        Language::Tsx => &TSX_PARSER,
        Language::Go => &GO_PARSER,
        Language::C => &C_PARSER,
        Language::Cpp => &CPP_PARSER,
    };
    parser_cell.with(|cell| {
        let mut guard = cell.borrow_mut();
        let parser = guard.get_or_insert_with(|| {
            let mut p = tree_sitter::Parser::new();
            p.set_language(&get_language(lang))
                .unwrap_or_else(|_| panic!("tree-sitter {:?} 语言初始化失败", lang));
            p
        });
        parser
            .parse(source, None)
            .expect("tree-sitter 解析失败")
    })
}

/// 从节点提取符号名
fn node_name(node: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())
        .map(|s| s.to_string())
}

/// 递归收集节点中所有感兴趣的符号
fn collect_symbols(node: tree_sitter::Node<'_>, source: &[u8], depth: usize, lang: Language) -> Vec<SymbolInfo> {
    if depth > 20 {
        return Vec::new();
    }
    let mut symbols = Vec::new();
    let kinds = interesting_kinds(lang);
    let kind = node.kind();
    if kinds.contains(&kind) && let Some(name) = node_name(node, source) {
        symbols.push(SymbolInfo {
            kind: kind.to_string(),
            name,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
        });
    }
    for child in node.children(&mut node.walk()) {
        symbols.extend(collect_symbols(child, source, depth + 1, lang));
    }
    symbols
}

/// 符号信息
struct SymbolInfo {
    kind: String,
    name: String,
    start_line: usize,
    end_line: usize,
    #[allow(dead_code)]
    start_byte: usize,
    #[allow(dead_code)]
    end_byte: usize,
}

/// 按名查找符号节点
fn find_symbol_node<'a>(
    node: tree_sitter::Node<'a>,
    source: &'a [u8],
    name: &str,
    depth: usize,
    lang: Language,
) -> Option<tree_sitter::Node<'a>> {
    if depth > 20 {
        return None;
    }
    let kinds = interesting_kinds(lang);
    let kind = node.kind();
    if kinds.contains(&kind) && let Some(n) = node_name(node, source) && n == name {
        return Some(node);
    }
    for child in node.children(&mut node.walk()) {
        if let Some(found) = find_symbol_node(child, source, name, depth + 1, lang) {
            return Some(found);
        }
    }
    None
}

/// 递归收集源码文件 (.rs / .java / .py / .js / .ts / .go / .c / .cpp ...)
fn collect_source_files(dir: &PathBuf, results: &mut Vec<PathBuf>, depth: usize) {
    if depth > 10 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name == "target" || name == ".git" || name == "node_modules" {
                continue;
            }
            collect_source_files(&path, results, depth + 1);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str())
            && matches!(ext, "rs" | "java" | "py" | "js" | "jsx" | "ts" | "tsx" | "go" | "c" | "h" | "cpp" | "hpp" | "cc" | "cxx") {
                results.push(path);
            }
    }
}

/// 从文件路径检测语言并解析,返回 (tree, lang)
fn parse_file(file_io: &FileIo, path: &str) -> Result<(tree_sitter::Tree, Language), adk_rust::AdkError> {
    let full = file_io.validate_path(path).map_err(|e| {
        adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR", format!("路径验证失败: {e}"))
    })?;
    let lang = detect_language(&full).ok_or_else(|| {
        adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT", format!("不支持的文件类型,仅支持 .rs/.java/.py/.js/.jsx/.ts/.tsx/.go/.c/.h/.cpp/.hpp/.cc/.cxx: {path}"))
    })?;
    let source = std::fs::read_to_string(&full).map_err(|e| {
        adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR", format!("读取文件失败: {e}"))
    })?;
    let tree = parse_source(&source, lang);
    Ok((tree, lang))
}

/// aft_outline: 列出文件的所有顶层符号 (多语言)
fn create_aft_outline_tool(file_io: FileIo) -> FunctionTool {
    FunctionTool::new(
        "aft_outline",
        "列出源码文件的所有顶层符号,返回符号名+行号范围。支持 .rs/.java/.py/.js/.jsx/.ts/.tsx/.go/.c/.h/.cpp/.hpp/.cc/.cxx。",
        move |_ctx: Arc<dyn adk_rust::tool::ToolContext>, args: Value| {
            let file_io = file_io.clone();
            Box::pin(async move {
                let path = args["path"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 path 参数")
                })?;
                let (tree, lang) = parse_file(&file_io, path)?;
                let full = file_io.validate_path(path).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR", format!("路径验证失败: {e}"))
                })?;
                let source = std::fs::read_to_string(&full).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR", format!("读取文件失败: {e}"))
                })?;
                let symbols = collect_symbols(tree.root_node(), source.as_bytes(), 0, lang);
                if symbols.is_empty() {
                    return Ok(serde_json::json!({ "output": "(无顶层符号)" }));
                }
                let lines: Vec<String> = symbols
                    .iter()
                    .map(|s| {
                        let kind_short = s.kind.trim_end_matches("_item").trim_end_matches("_declaration").trim_end_matches("_specifier").trim_end_matches("_definition");
                        format!("{kind_short} {} (line {}-{})", s.name, s.start_line, s.end_line)
                    })
                    .collect();
                Ok(serde_json::json!({ "output": lines.join("\n") }))
            })
        },
    )
}

/// aft_view_symbol: 查看文件中指定符号的完整定义源码
fn create_aft_view_symbol_tool(file_io: FileIo) -> FunctionTool {
    FunctionTool::new(
        "aft_view_symbol",
        "查看文件中指定符号的完整定义源码。参数: path (文件路径), symbol (符号名)。支持 .rs/.java/.py/.js/.jsx/.ts/.tsx/.go/.c/.h/.cpp/.hpp/.cc/.cxx。",
        move |_ctx: Arc<dyn adk_rust::tool::ToolContext>, args: Value| {
            let file_io = file_io.clone();
            Box::pin(async move {
                let path = args["path"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 path 参数")
                })?;
                let symbol = args["symbol"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 symbol 参数")
                })?;
                let (tree, lang) = parse_file(&file_io, path)?;
                let full = file_io.validate_path(path).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR", format!("路径验证失败: {e}"))
                })?;
                let source = std::fs::read_to_string(&full).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR", format!("读取文件失败: {e}"))
                })?;
                let node = find_symbol_node(tree.root_node(), source.as_bytes(), symbol, 0, lang)
                    .ok_or_else(|| {
                        adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("未找到符号: {symbol}"))
                    })?;
                let text = node.utf8_text(source.as_bytes()).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("提取文本失败: {e}"))
                })?;
                Ok(serde_json::json!({ "output": text.to_string() }))
            })
        },
    )
}

/// aft_edit_symbol: 替换文件中指定符号的完整定义
fn create_aft_edit_symbol_tool(file_io: FileIo) -> FunctionTool {
    FunctionTool::new(
        "aft_edit_symbol",
        "替换文件中指定符号的完整定义。参数: path (文件路径), symbol (符号名), content (新源码)。支持 .rs/.java/.py/.js/.jsx/.ts/.tsx/.go/.c/.h/.cpp/.hpp/.cc/.cxx。",
        move |_ctx: Arc<dyn adk_rust::tool::ToolContext>, args: Value| {
            let file_io = file_io.clone();
            Box::pin(async move {
                let path = args["path"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 path 参数")
                })?;
                let symbol = args["symbol"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 symbol 参数")
                })?;
                let new_content = args["content"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 content 参数")
                })?;
                let (tree, lang) = parse_file(&file_io, path)?;
                let full = file_io.validate_path(path).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR", format!("路径验证失败: {e}"))
                })?;
                let source = std::fs::read_to_string(&full).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR", format!("读取文件失败: {e}"))
                })?;
                let node = find_symbol_node(tree.root_node(), source.as_bytes(), symbol, 0, lang)
                    .ok_or_else(|| {
                        adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("未找到符号: {symbol}"))
                    })?;

                let start = node.start_byte();
                let end = node.end_byte();
                let mut new_source = source[..start].to_string();
                new_source.push_str(new_content);
                new_source.push_str(&source[end..]);

                // 验证新源码能通过 tree-sitter 解析
                let new_tree = parse_source(&new_source, lang);
                if new_tree.root_node().has_error() {
                    return Ok(serde_json::json!({
                        "error": "替换后的源码语法错误,操作已取消",
                        "success": false
                    }));
                }

                std::fs::write(&full, &new_source).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("写入文件失败: {e}"))
                })?;
                Ok(serde_json::json!({ "output": format!("已替换符号 {symbol}"), "success": true }))
            })
        },
    )
}

/// aft_search_symbols: 在 workspace 中搜索符号名匹配正则的符号
fn create_aft_search_symbols_tool(file_io: FileIo) -> FunctionTool {
    FunctionTool::new(
        "aft_search_symbols",
        "在 workspace 中搜索符号名匹配正则的符号。参数: pattern (正则), dir (可选,相对目录,默认根)。搜索 .rs/.java/.py/.js/.jsx/.ts/.tsx/.go/.c/.h/.cpp/.hpp/.cc/.cxx。",
        move |_ctx: Arc<dyn adk_rust::tool::ToolContext>, args: Value| {
            let file_io = file_io.clone();
            Box::pin(async move {
                let pattern_str = args["pattern"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 pattern 参数")
                })?;
                let dir = args["dir"].as_str().unwrap_or("");
                let regex = regex::Regex::new(pattern_str).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT",format!("无效正则: {e}"))
                })?;

                let search_dir = if dir.is_empty() {
                    file_io.workspace.clone()
                } else {
                    file_io.validate_path(dir).map_err(|e| {
                        adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("路径验证失败: {e}"))
                    })?
                };

                let mut results = Vec::new();
                collect_source_files(&search_dir, &mut results, 0);

                let mut matches = Vec::new();
                for file_path in &results {
                    let Some(lang) = detect_language(file_path) else { continue };
                    let rel = file_path
                        .strip_prefix(&file_io.workspace)
                        .unwrap_or(file_path)
                        .to_string_lossy()
                        .to_string();
                    let source = match std::fs::read_to_string(file_path) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    let tree = parse_source(&source, lang);
                    let symbols = collect_symbols(tree.root_node(), source.as_bytes(), 0, lang);
                    for sym in &symbols {
                        if regex.is_match(&sym.name) {
                            matches.push(format!(
                                "{}: {} {} (line {})",
                                rel,
                                sym.kind.trim_end_matches("_item").trim_end_matches("_declaration").trim_end_matches("_specifier").trim_end_matches("_definition"),
                                sym.name,
                                sym.start_line
                            ));
                        }
                    }
                }

                if matches.is_empty() {
                    return Ok(serde_json::json!({ "output": "(无匹配符号)" }));
                }
                Ok(serde_json::json!({ "output": matches.join("\n") }))
            })
        },
    )
}

/// aft_ast_replace: 在文件中用正则查找并替换,替换后验证语法
fn create_aft_ast_replace_tool(file_io: FileIo) -> FunctionTool {
    FunctionTool::new(
        "aft_ast_replace",
        "在文件中用正则查找并替换,替换后验证语法。参数: path, pattern (正则), replacement, flags (可选,如 \"i\")。支持 .rs/.java/.py/.js/.jsx/.ts/.tsx/.go/.c/.h/.cpp/.hpp/.cc/.cxx。",
        move |_ctx: Arc<dyn adk_rust::tool::ToolContext>, args: Value| {
            let file_io = file_io.clone();
            Box::pin(async move {
                let path = args["path"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 path 参数")
                })?;
                let pattern_str = args["pattern"].as_str().ok_or_else(|| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT","缺少 pattern 参数")
                })?;
                let replacement = args["replacement"].as_str().unwrap_or("");
                let flags = args["flags"].as_str().unwrap_or("");

                let regex = if flags.contains('i') {
                    regex::Regex::new(&format!("(?i){pattern_str}"))
                } else {
                    regex::Regex::new(pattern_str)
                }
                .map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::InvalidInput, "INVALID_ARGUMENT",format!("无效正则: {e}"))
                })?;

                let (_tree, lang) = parse_file(&file_io, path)?;
                let full = file_io.validate_path(path).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR", format!("路径验证失败: {e}"))
                })?;
                let source = std::fs::read_to_string(&full).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR", format!("读取文件失败: {e}"))
                })?;

                let new_source = regex.replace_all(&source, replacement).to_string();
                if new_source == source {
                    return Ok(serde_json::json!({ "output": "(无匹配,未修改)" }));
                }

                // 验证语法
                let new_tree = parse_source(&new_source, lang);
                if new_tree.root_node().has_error() {
                    return Ok(serde_json::json!({
                        "error": "替换后源码语法错误,操作已取消",
                        "success": false
                    }));
                }

                std::fs::write(&full, &new_source).map_err(|e| {
                    adk_rust::AdkError::new(adk_rust::ErrorComponent::Tool, adk_rust::ErrorCategory::Internal, "EXECUTION_ERROR",format!("写入文件失败: {e}"))
                })?;
                Ok(serde_json::json!({ "output": "替换完成", "success": true }))
            })
        },
    )
}