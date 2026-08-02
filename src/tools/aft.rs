//! AFT 代码感知工具 (P3.5): outline, view_symbol, edit_symbol, search_symbols, ast_replace
//!
//! 基于 tree-sitter 的 Rust 代码 AST 分析,提供符号级代码操作。
//! 工具名统一加 `aft_` 前缀避 namespace 冲突。

use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use regex::Regex;
use tree_sitter::{Node, Parser};

use crate::error::{DevnpcError, Result};
use crate::tools::file_io::FileIo;
use crate::tools::{Tool, ToolResult};

// ============================================================
// 共享 tree-sitter 引擎 (全局懒初始化)
// ============================================================

static ENGINE: Mutex<Option<Parser>> = Mutex::new(None);

fn parse_source(source: &str) -> tree_sitter::Tree {
    let mut guard = ENGINE.lock().unwrap();
    let parser = guard.get_or_insert_with(|| {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("tree-sitter Rust 语言初始化失败");
        p
    });
    parser
        .parse(source, None)
        .expect("tree-sitter 解析失败")
}

// ============================================================
// 符号提取
// ============================================================

/// 关注的 AST 节点类型 (Rust 顶层声明)
const INTERESTING_KINDS: &[&str] = &[
    "function_item",
    "struct_item",
    "enum_item",
    "trait_item",
    "impl_item",
    "mod_item",
    "const_item",
    "static_item",
    "type_item",
    "macro_definition",
    "union_item",
    "foreign_mod_item",
];

/// 符号信息
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct SymbolInfo {
    kind: String,
    name: String,
    start_line: usize,
    end_line: usize,
    start_byte: usize,
    end_byte: usize,
}

/// 从节点提取符号名 (尝试 field "name")
fn node_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())
        .map(|s| s.to_string())
}

/// 递归收集节点中所有感兴趣的符号
fn collect_symbols(node: Node<'_>, source: &[u8], depth: usize) -> Vec<SymbolInfo> {
    if depth > 20 {
        return Vec::new();
    }
    let mut symbols = Vec::new();
    let kind = node.kind();

    if INTERESTING_KINDS.contains(&kind) {
        if let Some(name) = node_name(node, source) {
            symbols.push(SymbolInfo {
                kind: kind.to_string(),
                name,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
            });
        }
    }

    for child in node.children(&mut node.walk()) {
        symbols.extend(collect_symbols(child, source, depth + 1));
    }
    symbols
}

/// 按名查找符号节点 (返回第一个匹配)
fn find_symbol_node<'a>(
    node: Node<'a>,
    source: &'a [u8],
    name: &str,
    depth: usize,
) -> Option<Node<'a>> {
    if depth > 20 {
        return None;
    }
    let kind = node.kind();
    if INTERESTING_KINDS.contains(&kind) {
        if let Some(n) = node_name(node, source) {
            if n == name {
                return Some(node);
            }
        }
    }
    for child in node.children(&mut node.walk()) {
        if let Some(found) = find_symbol_node(child, source, name, depth + 1) {
            return Some(found);
        }
    }
    None
}

// ============================================================
// Tool: aft_outline — 文件大纲
// ============================================================

pub struct AftOutlineTool {
    file_io: FileIo,
}

impl AftOutlineTool {
    pub fn new(file_io: FileIo) -> Self {
        Self { file_io }
    }
}

#[async_trait]
impl Tool for AftOutlineTool {
    fn name(&self) -> &str {
        "aft_outline"
    }
    fn description(&self) -> &str {
        "列出 Rust 文件的所有顶层符号 (函数/结构体/枚举/Trait 等),返回符号名+行号范围。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "相对 workspace 的文件路径"}
            },
            "required": ["path"]
        })
    }
    async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let path = args["path"].as_str().ok_or_else(|| DevnpcError::Tool {
            tool: "aft_outline".into(),
            msg: "缺少 path 参数".into(),
        })?;
        let full = self.file_io.validate_path(path)?;
        let source = std::fs::read_to_string(&full).map_err(|e| DevnpcError::Tool {
            tool: "aft_outline".into(),
            msg: format!("读取文件失败: {e}"),
        })?;
        let tree = parse_source(&source);
        let symbols = collect_symbols(tree.root_node(), source.as_bytes(), 0);
        if symbols.is_empty() {
            return Ok(ToolResult::ok("(无顶层符号)"));
        }
        let lines: Vec<String> = symbols
            .iter()
            .map(|s| {
                let kind_short = s.kind.trim_end_matches("_item").trim_end_matches("_invocation");
                format!("{kind_short} {} (line {}-{})", s.name, s.start_line, s.end_line)
            })
            .collect();
        Ok(ToolResult::ok(lines.join("\n")))
    }
}

// ============================================================
// Tool: aft_view_symbol — 查看符号定义
// ============================================================

pub struct AftViewSymbolTool {
    file_io: FileIo,
}

impl AftViewSymbolTool {
    pub fn new(file_io: FileIo) -> Self {
        Self { file_io }
    }
}

#[async_trait]
impl Tool for AftViewSymbolTool {
    fn name(&self) -> &str {
        "aft_view_symbol"
    }
    fn description(&self) -> &str {
        "查看文件中指定符号的完整定义源码。参数: path (文件路径), symbol (符号名)。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "symbol": {"type": "string"}
            },
            "required": ["path", "symbol"]
        })
    }
    async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let path = args["path"].as_str().ok_or_else(|| DevnpcError::Tool {
            tool: "aft_view_symbol".into(),
            msg: "缺少 path 参数".into(),
        })?;
        let symbol = args["symbol"].as_str().ok_or_else(|| DevnpcError::Tool {
            tool: "aft_view_symbol".into(),
            msg: "缺少 symbol 参数".into(),
        })?;
        let full = self.file_io.validate_path(path)?;
        let source = std::fs::read_to_string(&full).map_err(|e| DevnpcError::Tool {
            tool: "aft_view_symbol".into(),
            msg: format!("读取文件失败: {e}"),
        })?;
        let tree = parse_source(&source);
        let node = find_symbol_node(tree.root_node(), source.as_bytes(), symbol, 0).ok_or_else(
            || DevnpcError::Tool {
                tool: "aft_view_symbol".into(),
                msg: format!("未找到符号: {symbol}"),
            },
        )?;
        let text = node
            .utf8_text(source.as_bytes())
            .map_err(|e| DevnpcError::Tool {
                tool: "aft_view_symbol".into(),
                msg: format!("提取文本失败: {e}"),
            })?;
        Ok(ToolResult::ok(text.to_string()))
    }
}

// ============================================================
// Tool: aft_edit_symbol — 编辑符号定义
// ============================================================

pub struct AftEditSymbolTool {
    file_io: FileIo,
}

impl AftEditSymbolTool {
    pub fn new(file_io: FileIo) -> Self {
        Self { file_io }
    }
}

#[async_trait]
impl Tool for AftEditSymbolTool {
    fn name(&self) -> &str {
        "aft_edit_symbol"
    }
    fn description(&self) -> &str {
        "替换文件中指定符号的完整定义。参数: path (文件路径), symbol (符号名), content (新源码)。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "symbol": {"type": "string"},
                "content": {"type": "string", "description": "新的符号完整源码"}
            },
            "required": ["path", "symbol", "content"]
        })
    }
    async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let path = args["path"].as_str().ok_or_else(|| DevnpcError::Tool {
            tool: "aft_edit_symbol".into(),
            msg: "缺少 path 参数".into(),
        })?;
        let symbol = args["symbol"].as_str().ok_or_else(|| DevnpcError::Tool {
            tool: "aft_edit_symbol".into(),
            msg: "缺少 symbol 参数".into(),
        })?;
        let new_content = args["content"].as_str().ok_or_else(|| DevnpcError::Tool {
            tool: "aft_edit_symbol".into(),
            msg: "缺少 content 参数".into(),
        })?;
        let full = self.file_io.validate_path(path)?;
        let source = std::fs::read_to_string(&full).map_err(|e| DevnpcError::Tool {
            tool: "aft_edit_symbol".into(),
            msg: format!("读取文件失败: {e}"),
        })?;
        let tree = parse_source(&source);
        let node = find_symbol_node(tree.root_node(), source.as_bytes(), symbol, 0).ok_or_else(
            || DevnpcError::Tool {
                tool: "aft_edit_symbol".into(),
                msg: format!("未找到符号: {symbol}"),
            },
        )?;

        let start = node.start_byte();
        let end = node.end_byte();
        let mut new_source = source[..start].to_string();
        new_source.push_str(new_content);
        new_source.push_str(&source[end..]);

        // 验证新源码能通过 tree-sitter 解析
        let new_tree = parse_source(&new_source);
        if new_tree.root_node().has_error() {
            return Ok(ToolResult::err("替换后的源码语法错误,操作已取消"));
        }

        std::fs::write(&full, &new_source).map_err(|e| DevnpcError::Tool {
            tool: "aft_edit_symbol".into(),
            msg: format!("写入文件失败: {e}"),
        })?;
        Ok(ToolResult::ok(format!("已替换符号 {symbol}")))
    }
}

// ============================================================
// Tool: aft_search_symbols — 搜索符号
// ============================================================

pub struct AftSearchSymbolsTool {
    file_io: FileIo,
}

impl AftSearchSymbolsTool {
    pub fn new(file_io: FileIo) -> Self {
        Self { file_io }
    }
}

#[async_trait]
impl Tool for AftSearchSymbolsTool {
    fn name(&self) -> &str {
        "aft_search_symbols"
    }
    fn description(&self) -> &str {
        "在 workspace 中搜索符号名匹配正则的符号。参数: pattern (正则), dir (可选,相对目录,默认根)。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "符号名正则"},
                "dir": {"type": "string", "description": "搜索目录(相对),默认 workspace 根", "default": ""}
            },
            "required": ["pattern"]
        })
    }
    async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let pattern_str = args["pattern"].as_str().ok_or_else(|| DevnpcError::Tool {
            tool: "aft_search_symbols".into(),
            msg: "缺少 pattern 参数".into(),
        })?;
        let dir = args["dir"].as_str().unwrap_or("");
        let regex = Regex::new(pattern_str).map_err(|e| DevnpcError::Tool {
            tool: "aft_search_symbols".into(),
            msg: format!("无效正则: {e}"),
        })?;

        let search_dir = if dir.is_empty() {
            self.file_io.workspace.clone()
        } else {
            self.file_io.validate_path(dir)?
        };

        let mut results = Vec::new();
        collect_rs_files(&search_dir, &mut results, 0);

        let mut matches = Vec::new();
        for file_path in &results {
            let rel = file_path
                .strip_prefix(&self.file_io.workspace)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();
            let source = match std::fs::read_to_string(file_path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let tree = parse_source(&source);
            let symbols = collect_symbols(tree.root_node(), source.as_bytes(), 0);
            for sym in &symbols {
                if regex.is_match(&sym.name) {
                    matches.push(format!(
                        "{}: {} {} (line {})",
                        rel,
                        sym.kind.trim_end_matches("_item"),
                        sym.name,
                        sym.start_line
                    ));
                }
            }
        }

        if matches.is_empty() {
            return Ok(ToolResult::ok("(无匹配符号)"));
        }
        Ok(ToolResult::ok(matches.join("\n")))
    }
}

/// 递归收集 .rs 文件
fn collect_rs_files(dir: &PathBuf, results: &mut Vec<PathBuf>, depth: usize) {
    if depth > 10 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // 跳过 target 和 .git
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name == "target" || name == ".git" || name == "node_modules" {
                continue;
            }
            collect_rs_files(&path, results, depth + 1);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            results.push(path);
        }
    }
}

// ============================================================
// Tool: aft_ast_replace — AST 模式替换 (基于正则)
// ============================================================

pub struct AftAstReplaceTool {
    file_io: FileIo,
}

impl AftAstReplaceTool {
    pub fn new(file_io: FileIo) -> Self {
        Self { file_io }
    }
}

#[async_trait]
impl Tool for AftAstReplaceTool {
    fn name(&self) -> &str {
        "aft_ast_replace"
    }
    fn description(&self) -> &str {
        "在文件中用正则查找并替换,替换后验证语法。参数: path, pattern (正则), replacement, flags (可选,如 \"i\")。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "pattern": {"type": "string", "description": "搜索正则"},
                "replacement": {"type": "string", "description": "替换文本"},
                "flags": {"type": "string", "description": "正则标志,如 \"i\" 忽略大小写", "default": ""}
            },
            "required": ["path", "pattern", "replacement"]
        })
    }
    async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let path = args["path"].as_str().ok_or_else(|| DevnpcError::Tool {
            tool: "aft_ast_replace".into(),
            msg: "缺少 path 参数".into(),
        })?;
        let pattern_str = args["pattern"].as_str().ok_or_else(|| DevnpcError::Tool {
            tool: "aft_ast_replace".into(),
            msg: "缺少 pattern 参数".into(),
        })?;
        let replacement = args["replacement"].as_str().unwrap_or("");
        let flags = args["flags"].as_str().unwrap_or("");

        let regex = if flags.contains('i') {
            Regex::new(&format!("(?i){pattern_str}"))
        } else {
            Regex::new(pattern_str)
        }
        .map_err(|e| DevnpcError::Tool {
            tool: "aft_ast_replace".into(),
            msg: format!("无效正则: {e}"),
        })?;

        let full = self.file_io.validate_path(path)?;
        let source = std::fs::read_to_string(&full).map_err(|e| DevnpcError::Tool {
            tool: "aft_ast_replace".into(),
            msg: format!("读取文件失败: {e}"),
        })?;

        let new_source = regex.replace_all(&source, replacement).to_string();

        if new_source == source {
            return Ok(ToolResult::ok("(无匹配,未修改)"));
        }

        // 验证语法
        let new_tree = parse_source(&new_source);
        if new_tree.root_node().has_error() {
            return Ok(ToolResult::err("替换后源码语法错误,操作已取消"));
        }

        std::fs::write(&full, &new_source).map_err(|e| DevnpcError::Tool {
            tool: "aft_ast_replace".into(),
            msg: format!("写入文件失败: {e}"),
        })?;
        Ok(ToolResult::ok("替换完成"))
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn aft_outline_returns_symbols() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.rs"),
            r#"
fn hello() {}
struct MyStruct { x: i32 }
enum MyEnum { A, B }
trait MyTrait { fn foo(); }
impl MyTrait for MyStruct {}
"#,
        )
        .unwrap();
        let tool = AftOutlineTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"path": "test.rs"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello"));
        assert!(result.output.contains("MyStruct"));
        assert!(result.output.contains("MyEnum"));
        assert!(result.output.contains("MyTrait"));
    }

    #[tokio::test]
    async fn aft_outline_empty_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("empty.rs"), "").unwrap();
        let tool = AftOutlineTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"path": "empty.rs"}))
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output, "(无顶层符号)");
    }

    #[tokio::test]
    async fn aft_view_symbol_returns_function_body() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            r#"
fn helper() -> i32 {
    42
}
"#,
        )
        .unwrap();
        let tool = AftViewSymbolTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"path": "lib.rs", "symbol": "helper"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("fn helper"));
        assert!(result.output.contains("42"));
    }

    #[tokio::test]
    async fn aft_view_symbol_not_found() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn foo() {}").unwrap();
        let tool = AftViewSymbolTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"path": "lib.rs", "symbol": "nonexistent"}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn aft_edit_symbol_replaces_symbol() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            r#"
fn old_func() -> i32 {
    1
}
"#,
        )
        .unwrap();
        let tool = AftEditSymbolTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({
                "path": "lib.rs",
                "symbol": "old_func",
                "content": "fn new_func() -> i32 {\n    999\n}"
            }))
            .await
            .unwrap();
        assert!(result.success);
        let content = std::fs::read_to_string(dir.path().join("lib.rs")).unwrap();
        assert!(content.contains("new_func"));
        assert!(content.contains("999"));
        assert!(!content.contains("old_func"));
    }

    #[tokio::test]
    async fn aft_edit_symbol_rejects_broken_syntax() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            r#"
fn ok() -> i32 { 1 }
"#,
        )
        .unwrap();
        let tool = AftEditSymbolTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({
                "path": "lib.rs",
                "symbol": "ok",
                "content": "fn broken( -> i32 { 1 }"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("语法错误"));
    }

    #[tokio::test]
    async fn aft_search_symbols_finds_by_pattern() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("math.rs"),
            r#"
fn add(a: i32, b: i32) -> i32 { a + b }
fn sub(a: i32, b: i32) -> i32 { a - b }
struct Point { x: f64, y: f64 }
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("main.rs"),
            r#"
fn main() {}
"#,
        )
        .unwrap();
        let tool = AftSearchSymbolsTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({"pattern": "add|sub"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("add"));
        assert!(result.output.contains("sub"));
        assert!(!result.output.contains("Point"));
    }

    #[tokio::test]
    async fn aft_ast_replace_replaces_text() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            r#"fn old_name() -> i32 { 1 }"#,
        )
        .unwrap();
        let tool = AftAstReplaceTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({
                "path": "lib.rs",
                "pattern": "old_name",
                "replacement": "new_name"
            }))
            .await
            .unwrap();
        assert!(result.success);
        let content = std::fs::read_to_string(dir.path().join("lib.rs")).unwrap();
        assert!(content.contains("new_name"));
    }

    #[tokio::test]
    async fn aft_ast_replace_no_match() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn foo() {}").unwrap();
        let tool = AftAstReplaceTool::new(FileIo::new(dir.path()));
        let result = tool
            .call(&serde_json::json!({
                "path": "lib.rs",
                "pattern": "bar",
                "replacement": "baz"
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output, "(无匹配,未修改)");
    }
}