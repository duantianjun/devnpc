//! NPC 间通信 (P7 完整实现: GitLab 评论总线)
//!
//! 协议头: [devnpc:handoff] ... [/devnpc:handoff]
//!
//! 通信通过 GitLab Issue 评论进行,每个 NPC 读取 Issue 评论中的 handoff 消息,
//! 了解当前进度和需要执行的任务。

use crate::error::Result;

/// Handoff 消息 (P7)
#[derive(Debug, Clone)]
pub struct Handoff {
    /// 发送者角色名
    pub from: String,
    /// 接收者角色名列表
    pub to: Vec<String>,
    /// 信号: ready / done / review / merge
    pub signal: String,
    /// 消息体 (任务详情或执行结果)
    pub payload: String,
}

/// 解析 handoff 消息 (P7 实现)
///
/// 从 GitLab Issue 评论体中提取 [devnpc:handoff] 协议消息。
/// 如果没有匹配的 handoff 协议,返回 None。
pub fn parse_handoff(body: &str) -> Result<Option<Handoff>> {
    let start = "[devnpc:handoff]";
    let end = "[/devnpc:handoff]";

    let Some(start_pos) = body.find(start) else {
        return Ok(None);
    };
    let content_start = start_pos + start.len();
    let Some(end_pos) = body[content_start..].find(end) else {
        return Ok(None);
    };
    let content = body[content_start..content_start + end_pos].trim();

    // 解析 JSON 格式: {"from":"PM","to":["dev"],"signal":"ready","payload":"..."}
    let parsed: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| crate::error::DevnpcError::Config(format!("handoff JSON 解析失败: {e}")))?;

    let from = parsed
        .get("from")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            crate::error::DevnpcError::Config("handoff 缺少 from 字段".into())
        })?
        .to_string();

    let to: Vec<String> = parsed
        .get("to")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let signal = parsed
        .get("signal")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            crate::error::DevnpcError::Config("handoff 缺少 signal 字段".into())
        })?
        .to_string();

    let payload = parsed
        .get("payload")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(Some(Handoff {
        from,
        to,
        signal,
        payload,
    }))
}

/// 构建 handoff 消息 (P7 实现)
///
/// 生成符合 [devnpc:handoff] 协议的评论正文。
pub fn build_handoff(from: &str, to: &[String], signal: &str, payload: &str) -> String {
    let to_json: Vec<String> = to.iter().map(|t| format!("\"{t}\"")).collect();
    let to_array = to_json.join(",");
    format!(
        "[devnpc:handoff]{{\"from\":\"{from}\",\"to\":[{to_array}],\"signal\":\"{signal}\",\"payload\":{payload_json}}}[/devnpc:handoff]",
        payload_json = serde_json::to_string(payload).unwrap_or_else(|_| format!("\"{}\"", payload))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_handoff_returns_none_when_no_marker() {
        let body = "这是一个普通的评论";
        let result = parse_handoff(body).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_handoff_extracts_fields() {
        let body = "[devnpc:handoff]{\"from\":\"PM\",\"to\":[\"dev\"],\"signal\":\"ready\",\"payload\":\"实现登录功能\"}[/devnpc:handoff]";
        let result = parse_handoff(body).unwrap();
        assert!(result.is_some());
        let handoff = result.unwrap();
        assert_eq!(handoff.from, "PM");
        assert_eq!(handoff.to, vec!["dev"]);
        assert_eq!(handoff.signal, "ready");
        assert_eq!(handoff.payload, "实现登录功能");
    }

    #[test]
    fn parse_handoff_handles_multiple_recipients() {
        let body = "[devnpc:handoff]{\"from\":\"PM\",\"to\":[\"dev\",\"test\"],\"signal\":\"review\",\"payload\":\"请代码审查和测试\"}[/devnpc:handoff]";
        let result = parse_handoff(body).unwrap();
        assert!(result.is_some());
        let handoff = result.unwrap();
        assert_eq!(handoff.to, vec!["dev", "test"]);
        assert_eq!(handoff.signal, "review");
    }

    #[test]
    fn parse_handoff_handles_extra_text_around_marker() {
        let body = "一些前置文字\n[devnpc:handoff]{\"from\":\"PM\",\"to\":[\"dev\"],\"signal\":\"ready\",\"payload\":\"任务\"}[/devnpc:handoff]\n后续文字";
        let result = parse_handoff(body).unwrap();
        assert!(result.is_some());
        let handoff = result.unwrap();
        assert_eq!(handoff.from, "PM");
        assert_eq!(handoff.signal, "ready");
    }

    #[test]
    fn parse_handoff_returns_error_on_invalid_json() {
        let body = "[devnpc:handoff]{invalid json}[/devnpc:handoff]";
        let result = parse_handoff(body);
        assert!(result.is_err());
    }

    #[test]
    fn build_handoff_creates_valid_format() {
        let to = vec!["dev".to_string(), "test".to_string()];
        let msg = build_handoff("PM", &to, "ready", "实现登录功能");
        assert!(msg.contains("[devnpc:handoff]"));
        assert!(msg.contains("[/devnpc:handoff]"));
        assert!(msg.contains("\"from\":\"PM\""));
        assert!(msg.contains("\"to\":[\"dev\",\"test\"]"));
        assert!(msg.contains("\"signal\":\"ready\""));
        assert!(msg.contains("实现登录功能"));
    }

    #[test]
    fn build_then_parse_roundtrip() {
        let to = vec!["dev".to_string()];
        let msg = build_handoff("PM", &to, "done", "功能已完成");
        let result = parse_handoff(&msg).unwrap().unwrap();
        assert_eq!(result.from, "PM");
        assert_eq!(result.to, vec!["dev"]);
        assert_eq!(result.signal, "done");
        assert_eq!(result.payload, "功能已完成");
    }
}