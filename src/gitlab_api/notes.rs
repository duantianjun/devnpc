//! 评论操作工具函数
//!
//! 提供 Note 列表的纯函数工具:按作者过滤、提取 @提及、格式化摘要。
//! 这些函数不依赖 `GitlabApi` trait,便于在聚合层和展示层复用。

use crate::gitlab_api::Note;

/// 按作者 username 过滤评论,返回匹配的 Note 引用
pub fn filter_by_author<'a>(notes: &'a [Note], username: &str) -> Vec<&'a Note> {
    notes
        .iter()
        .filter(|n| n.author.username == username)
        .collect()
}

/// 从评论文本中提取 @提及 的 username (不含 @)
///
/// 规则: `@` 后跟字母/数字/下划线/连字符的序列,长度 >= 1。
/// 去重保序。
pub fn extract_mentions(body: &str) -> Vec<String> {
    let mut result = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() {
                let c = bytes[j];
                if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                    j += 1;
                } else {
                    break;
                }
            }
            if j > start
                && let Ok(name) = std::str::from_utf8(&bytes[start..j])
                    && !result.iter().any(|s: &String| s == name) {
                        result.push(name.to_string());
                    }
            i = j;
        } else {
            i += 1;
        }
    }
    result
}

/// 把 Note 列表格式化为单行摘要 (用于日志/调试)
///
/// 格式: `[{username}] {body_first_line} | ...`,每条一行。
pub fn format_summary(notes: &[Note]) -> String {
    notes
        .iter()
        .map(|n| {
            let first_line = n.body.lines().next().unwrap_or("");
            format!("[{}] {}", n.author.username, first_line)
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// 判断评论是否包含指令关键词 (如 `/fix`、`/close`、`/retry`)
pub fn contains_command(note: &Note, command: &str) -> bool {
    // 命令应以行首或空格开头,避免误匹配子串
    let body = note.body.as_str();
    if body.starts_with(command) {
        return true;
    }
    // 检查每个 token 是否等于 command
    body.split_whitespace().any(|tok| tok == command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab_api::NoteAuthor;

    fn make_note(id: u64, body: &str, username: &str) -> Note {
        Note {
            id,
            body: body.into(),
            author: NoteAuthor {
                id: 1,
                username: username.into(),
                name: username.into(),
            },
            created_at: "2026-08-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn filter_by_author_returns_matching_notes() {
        let notes = vec![
            make_note(1, "hi", "alice"),
            make_note(2, "hey", "bob"),
            make_note(3, "hello", "alice"),
        ];
        let alice_notes = filter_by_author(&notes, "alice");
        assert_eq!(alice_notes.len(), 2);
        assert_eq!(alice_notes[0].id, 1);
        assert_eq!(alice_notes[1].id, 3);
    }

    #[test]
    fn filter_by_author_returns_empty_when_no_match() {
        let notes = vec![make_note(1, "hi", "alice")];
        assert!(filter_by_author(&notes, "charlie").is_empty());
    }

    #[test]
    fn extract_mentions_finds_multiple_unique() {
        let mentions = extract_mentions("@alice 请看 @bob 的修改,cc @alice");
        assert_eq!(mentions, vec!["alice", "bob"]);
    }

    #[test]
    fn extract_mentions_handles_edge_cases() {
        // 末尾 @
        assert_eq!(extract_mentions("ping @alice"), vec!["alice"]);
        // @ 后无字符
        assert!(extract_mentions("end @").is_empty());
        // 多行
        assert_eq!(
            extract_mentions("line1 @x\nline2 @y"),
            vec!["x", "y"]
        );
        // 中文混合
        assert_eq!(extract_mentions("@bob 你好"), vec!["bob"]);
    }

    #[test]
    fn format_summary_joins_first_lines() {
        let notes = vec![
            make_note(1, "第一行\n第二行", "alice"),
            make_note(2, "仅一行", "bob"),
        ];
        let summary = format_summary(&notes);
        assert_eq!(summary, "[alice] 第一行 | [bob] 仅一行");
    }

    #[test]
    fn format_summary_empty_returns_empty_string() {
        assert_eq!(format_summary(&[]), "");
    }

    #[test]
    fn contains_command_matches_at_start() {
        let note = make_note(1, "/fix the login bug", "alice");
        assert!(contains_command(&note, "/fix"));
        assert!(!contains_command(&note, "/close"));
    }

    #[test]
    fn contains_command_matches_as_token() {
        let note = make_note(1, "please /retry the pipeline", "alice");
        assert!(contains_command(&note, "/retry"));
    }

    #[test]
    fn contains_command_does_not_match_substring() {
        let note = make_note(1, "this is a /fixup commit", "alice");
        // /fixup 不应匹配 /fix (作为完整 token 时)
        assert!(!contains_command(&note, "/fix"));
    }
}
