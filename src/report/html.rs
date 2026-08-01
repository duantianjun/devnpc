//! HTML 报告生成 (P4 完整实现,基于 askama)

/// 报告数据 (P4 完整实现)
#[derive(Debug, Clone)]
pub struct ReportData {
    pub status: String,
    pub duration_secs: u64,
    pub token_total: u64,
    pub llm_calls: u32,
    pub tool_calls: u32,
    pub ci_retries: u8,
    pub mr_url: Option<String>,
    pub summary: String,
}

impl Default for ReportData {
    fn default() -> Self {
        Self {
            status: "unknown".into(),
            duration_secs: 0,
            token_total: 0,
            llm_calls: 0,
            tool_calls: 0,
            ci_retries: 0,
            mr_url: None,
            summary: String::new(),
        }
    }
}

/// 生成 HTML (P4 完整实现,改用 askama 模板)
pub fn generate_html(data: &ReportData) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="zh">
<head><meta charset="UTF-8"><title>devnpc 报告</title></head>
<body>
<h1>devnpc 运维报告</h1>
<p>状态: {status}</p>
<p>耗时: {duration_secs}s</p>
<p>Token: {token_total}</p>
<p>LLM 调用: {llm_calls}</p>
<p>工具调用: {tool_calls}</p>
<p>CI 重试: {ci_retries}</p>
<p>摘要: {summary}</p>
</body>
</html>"#,
        status = data.status,
        duration_secs = data.duration_secs,
        token_total = data.token_total,
        llm_calls = data.llm_calls,
        tool_calls = data.tool_calls,
        ci_retries = data.ci_retries,
        summary = data.summary,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_html_contains_status() {
        let data = ReportData {
            status: "success".into(),
            ..Default::default()
        };
        let html = generate_html(&data);
        assert!(html.contains("success"));
        assert!(html.contains("<!DOCTYPE html>"));
    }
}
