//! HTML 报告生成
//!
//! 基于 ReportData 生成带有完整 overview / task / trajectory / cost / output 的 HTML 报告。

use crate::report::collector::ReportData;

/// 生成 HTML 报告
pub fn generate_html(data: &ReportData) -> String {
    let status_color = if data.status == "passed" {
        "var(--color-success)"
    } else if data.status.starts_with("failed") || data.status.starts_with("timeout") {
        "var(--color-error)"
    } else {
        "var(--color-warning)"
    };

    let status_icon = if data.status == "passed" {
        "&#10004;"
    } else if data.status.starts_with("failed") || data.status.starts_with("timeout") {
        "&#10008;"
    } else {
        "&#9888;"
    };

    let trajectory_rows: String = data
        .trajectory
        .events
        .iter()
        .enumerate()
        .map(|(i, ev)| {
            let (icon, row_class) = match ev.kind.as_str() {
                "llm_call" => ("&#129302;", "event-llm"),
                "tool_call" if ev.success == Some(true) => ("&#9889;", "event-tool-ok"),
                "tool_call" => ("&#9889;", "event-tool-err"),
                "deviation" => ("&#9888;", "event-deviation"),
                _ => ("&#9679;", ""),
            };
            format!(
                r#"<tr class="{row_class}">
                    <td class="event-idx">{}</td>
                    <td class="event-icon">{icon}</td>
                    <td class="event-detail">{}</td>
                </tr>"#,
                i + 1,
                ev.detail
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mr_section = if let Some(iid) = data.mr_iid {
        format!(
            r#"<div class="output-item">
                <span class="output-label">MR IID</span>
                <span class="output-value">!{iid}</span>
            </div>"#
        )
    } else {
        String::new()
    };

    let pipeline_section = if let Some(pid) = data.pipeline_id {
        format!(
            r#"<div class="output-item">
                <span class="output-label">Pipeline ID</span>
                <span class="output-value">#{pid}</span>
            </div>"#
        )
    } else {
        String::new()
    };

    let mr_url_section = data.mr_url.as_ref().map(|url| {
        format!(
            r#"<div class="output-item">
                <span class="output-label">MR 链接</span>
                <span class="output-value"><a href="{url}" target="_blank">{url}</a></span>
            </div>"#
        )
    }).unwrap_or_default();

    let ci_url_section = data.ci_url.as_ref().map(|url| {
        format!(
            r#"<div class="output-item">
                <span class="output-label">CI 链接</span>
                <span class="output-value"><a href="{url}" target="_blank">{url}</a></span>
            </div>"#
        )
    }).unwrap_or_default();

    // Team 协作流程可视化 (仅在 Team 模式下渲染)
    let team_section = if data.team_steps.is_empty() {
        String::new()
    } else {
        let steps_html: String = data
            .team_steps
            .iter()
            .enumerate()
            .map(|(i, step)| {
                let signals_html = if step.signals.is_empty() {
                    String::new()
                } else {
                    let sigs: Vec<String> = step
                        .signals
                        .iter()
                        .map(|s| format!(r#"<span class="signal-badge">{s}</span>"#))
                        .collect();
                    format!(r#"<div class="team-signals">{}</div>"#, sigs.join(""))
                };
                let role_icon = match step.role.as_str() {
                    "pm" => "&#128203;",
                    "developer" | "dev" => "&#128187;",
                    "tester" | "test" => "&#9989;",
                    _ => "&#9679;",
                };
                format!(
                    r#"<div class="team-step">
                        <div class="team-step-header">
                            <span class="team-step-icon">{role_icon}</span>
                            <span class="team-step-role">{}</span>
                            <span class="team-step-idx">步骤 {}</span>
                        </div>
                        <div class="team-step-instruction">指令: {}</div>
                        <div class="team-step-output">{}</div>
                        {signals_html}
                    </div>"#,
                    html_escape(&step.role),
                    i + 1,
                    html_escape(&step.instruction),
                    html_escape(&step.output),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            r#"
<!-- Team Collaboration -->
<div class="section">
    <h2>Team 协作流程 <span style="font-size:0.8rem;color:var(--color-text-secondary);font-weight:normal;">({} 个步骤)</span></h2>
    <div class="team-timeline">
        {steps_html}
    </div>
</div>
"#,
            data.team_steps.len()
        )
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="zh">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>devnpc 运维报告</title>
<style>
    :root {{
        --color-bg: #0d1117;
        --color-surface: #161b22;
        --color-border: #30363d;
        --color-text: #c9d1d9;
        --color-text-secondary: #8b949e;
        --color-success: #3fb950;
        --color-error: #f85149;
        --color-warning: #d29922;
        --color-accent: #58a6ff;
        --font-mono: 'SF Mono', 'Cascadia Code', 'Fira Code', monospace;
        --font-sans: -apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif;
    }}
    * {{ margin: 0; padding: 0; box-sizing: border-box; }}
    body {{
        background: var(--color-bg);
        color: var(--color-text);
        font-family: var(--font-sans);
        line-height: 1.6;
        padding: 2rem;
        max-width: 960px;
        margin: 0 auto;
    }}
    h1 {{ font-size: 1.6rem; margin-bottom: 1.5rem; color: var(--color-accent); }}
    h2 {{ font-size: 1.2rem; margin: 1.5rem 0 0.8rem; padding-bottom: 0.4rem; border-bottom: 1px solid var(--color-border); }}
    .section {{ background: var(--color-surface); border: 1px solid var(--color-border); border-radius: 8px; padding: 1.2rem; margin-bottom: 1.2rem; }}
    .status-badge {{
        display: inline-flex; align-items: center; gap: 0.4rem;
        padding: 0.3rem 0.8rem; border-radius: 20px;
        font-weight: 600; font-size: 0.9rem;
        background: {status_color}; color: #0d1117;
    }}
    .overview-grid {{
        display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
        gap: 1rem; margin-top: 1rem;
    }}
    .overview-item {{
        background: rgba(255,255,255,0.03); padding: 0.6rem 0.8rem;
        border-radius: 6px; border: 1px solid var(--color-border);
    }}
    .overview-label {{ font-size: 0.75rem; color: var(--color-text-secondary); text-transform: uppercase; letter-spacing: 0.5px; }}
    .overview-value {{ font-size: 1.1rem; font-weight: 600; margin-top: 0.2rem; }}
    table {{ width: 100%; border-collapse: collapse; }}
    th, td {{ text-align: left; padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--color-border); font-size: 0.9rem; }}
    th {{ color: var(--color-text-secondary); font-weight: 500; font-size: 0.8rem; text-transform: uppercase; }}
    .event-llm {{ background: rgba(88,166,255,0.05); }}
    .event-tool-ok {{ background: rgba(63,185,80,0.05); }}
    .event-tool-err {{ background: rgba(248,81,73,0.08); }}
    .event-deviation {{ background: rgba(210,153,34,0.08); }}
    .event-idx {{ color: var(--color-text-secondary); width: 2.5rem; text-align: right; font-family: var(--font-mono); font-size: 0.8rem; }}
    .event-icon {{ width: 2rem; text-align: center; font-size: 1rem; }}
    .event-detail {{ font-family: var(--font-mono); font-size: 0.85rem; }}
    .cost-grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 0.8rem; }}
    .cost-item {{ background: rgba(255,255,255,0.03); padding: 0.5rem 0.8rem; border-radius: 6px; }}
    .cost-label {{ font-size: 0.75rem; color: var(--color-text-secondary); }}
    .cost-value {{ font-size: 1rem; font-weight: 600; }}
    .task-block {{
        background: rgba(255,255,255,0.03); padding: 0.8rem; border-radius: 6px;
        font-family: var(--font-mono); font-size: 0.85rem; white-space: pre-wrap;
        border-left: 3px solid var(--color-accent);
    }}
    .output-item {{
        display: flex; align-items: center; gap: 0.8rem; padding: 0.4rem 0;
    }}
    .output-label {{ color: var(--color-text-secondary); font-size: 0.85rem; min-width: 100px; }}
    .output-value {{ font-family: var(--font-mono); font-size: 0.9rem; }}
    a {{ color: var(--color-accent); text-decoration: none; }}
    a:hover {{ text-decoration: underline; }}
    .footer {{ margin-top: 2rem; text-align: center; color: var(--color-text-secondary); font-size: 0.8rem; }}
    .team-timeline {{ display: flex; flex-direction: column; gap: 0.8rem; }}
    .team-step {{
        background: rgba(255,255,255,0.03); border: 1px solid var(--color-border);
        border-radius: 6px; padding: 0.8rem; border-left: 3px solid var(--color-accent);
    }}
    .team-step-header {{ display: flex; align-items: center; gap: 0.5rem; margin-bottom: 0.4rem; }}
    .team-step-icon {{ font-size: 1.1rem; }}
    .team-step-role {{ font-weight: 600; color: var(--color-accent); text-transform: uppercase; font-size: 0.85rem; }}
    .team-step-idx {{ color: var(--color-text-secondary); font-size: 0.75rem; margin-left: auto; }}
    .team-step-instruction {{ font-size: 0.8rem; color: var(--color-text-secondary); margin-bottom: 0.4rem; font-family: var(--font-mono); }}
    .team-step-output {{ font-family: var(--font-mono); font-size: 0.85rem; white-space: pre-wrap; }}
    .team-signals {{ margin-top: 0.4rem; display: flex; gap: 0.4rem; flex-wrap: wrap; }}
    .signal-badge {{
        background: rgba(63,185,80,0.15); color: var(--color-success);
        padding: 0.15rem 0.5rem; border-radius: 10px; font-size: 0.75rem; font-weight: 500;
    }}
</style>
</head>
<body>

<h1>devnpc 运维报告</h1>

<!-- Overview -->
<div class="section">
    <h2>概览</h2>
    <div style="display:flex;align-items:center;gap:1rem;margin-bottom:0.5rem;">
        <span class="status-badge">{status_icon} {status}</span>
        <span style="color:var(--color-text-secondary);font-size:0.9rem;">{duration}</span>
    </div>
    <div class="overview-grid">
        <div class="overview-item">
            <div class="overview-label">Token 总量</div>
            <div class="overview-value">{token_total}</div>
        </div>
        <div class="overview-item">
            <div class="overview-label">LLM 调用</div>
            <div class="overview-value">{llm_calls}</div>
        </div>
        <div class="overview-item">
            <div class="overview-label">工具调用</div>
            <div class="overview-value">{tool_calls}</div>
        </div>
        <div class="overview-item">
            <div class="overview-label">CI 重试</div>
            <div class="overview-value">{ci_retries}</div>
        </div>
        <div class="overview-item">
            <div class="overview-label">开始时间</div>
            <div class="overview-value" style="font-size:0.85rem;font-family:var(--font-mono);">{started_at}</div>
        </div>
        <div class="overview-item">
            <div class="overview-label">结束时间</div>
            <div class="overview-value" style="font-size:0.85rem;font-family:var(--font-mono);">{finished_at}</div>
        </div>
    </div>
</div>

<!-- Task -->
<div class="section">
    <h2>任务</h2>
    <div class="task-block">{task_description}</div>
</div>
{team_section}
<!-- Trajectory Timeline -->
<div class="section">
    <h2>执行轨迹 <span style="font-size:0.8rem;color:var(--color-text-secondary);font-weight:normal;">({trajectory_len} 个事件)</span></h2>
    <table>
        <thead>
            <tr>
                <th style="width:2.5rem;text-align:right;">#</th>
                <th style="width:2rem;"></th>
                <th>事件</th>
            </tr>
        </thead>
        <tbody>
            {trajectory_rows}
        </tbody>
    </table>
</div>

<!-- Cost -->
<div class="section">
    <h2>成本估算</h2>
    <div class="cost-grid">
        <div class="cost-item">
            <div class="cost-label">输入 Token</div>
            <div class="cost-value">{input_tokens}</div>
        </div>
        <div class="cost-item">
            <div class="cost-label">输出 Token</div>
            <div class="cost-value">{output_tokens}</div>
        </div>
        <div class="cost-item">
            <div class="cost-label">估算费用 (USD)</div>
            <div class="cost-value" style="color:var(--color-warning);">${cost_usd:.6}</div>
        </div>
    </div>
</div>

<!-- Output -->
<div class="section">
    <h2>产出</h2>
    {mr_section}
    {pipeline_section}
    {mr_url_section}
    {ci_url_section}
    <div class="output-item" style="margin-top:0.6rem;">
        <span class="output-label">摘要</span>
        <span class="output-value">{summary}</span>
    </div>
</div>

<div class="footer">
    Generated by devnpc &middot; {finished_at}
</div>

</body>
</html>"#,
        status = data.status,
        duration = format_duration(data.duration_secs),
        token_total = data.token_total,
        llm_calls = data.llm_calls,
        tool_calls = data.tool_calls,
        ci_retries = data.ci_retries,
        started_at = data.started_at,
        finished_at = data.finished_at,
        task_description = html_escape(&data.task_description),
        trajectory_len = data.trajectory.events.len(),
        trajectory_rows = trajectory_rows,
        input_tokens = data.cost_estimate.input_tokens,
        output_tokens = data.cost_estimate.output_tokens,
        cost_usd = data.cost_estimate.estimated_cost_usd,
        mr_section = mr_section,
        pipeline_section = pipeline_section,
        mr_url_section = mr_url_section,
        ci_url_section = ci_url_section,
        team_section = team_section,
        summary = html_escape(&data.summary),
    )
}

/// 将秒数格式化为可读字符串
fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs} 秒")
    } else if secs < 3600 {
        format!("{} 分 {} 秒", secs / 60, secs % 60)
    } else {
        format!(
            "{} 时 {} 分 {} 秒",
            secs / 3600,
            (secs % 3600) / 60,
            secs % 60
        )
    }
}

/// 简单的 HTML 转义 (防止 XSS)
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::collector::{CostEstimate, ReportData, TrajectorySummary};

    fn make_sample_data() -> ReportData {
        ReportData {
            status: "passed".into(),
            duration_secs: 125,
            token_total: 3500,
            llm_calls: 5,
            tool_calls: 12,
            ci_retries: 1,
            mr_url: Some("https://gitlab.test/mrs/42".into()),
            ci_url: Some("https://gitlab.test/pipelines/100".into()),
            summary: "自动修复了编译错误".into(),
            task_description: "修复登录页面 bug".into(),
            trajectory: TrajectorySummary {
                events: vec![
                    crate::report::collector::TrajectoryEventSummary {
                        kind: "llm_call".into(),
                        detail: "LLM 调用 (iteration #0)".into(),
                        success: Some(true),
                    },
                    crate::report::collector::TrajectoryEventSummary {
                        kind: "tool_call".into(),
                        detail: "工具: read_file".into(),
                        success: Some(true),
                    },
                    crate::report::collector::TrajectoryEventSummary {
                        kind: "tool_call".into(),
                        detail: "工具: write_file".into(),
                        success: Some(true),
                    },
                    crate::report::collector::TrajectoryEventSummary {
                        kind: "deviation".into(),
                        detail: "SOP 偏离".into(),
                        success: None,
                    },
                ],
            },
            cost_estimate: CostEstimate {
                input_tokens: 2500,
                output_tokens: 1000,
                estimated_cost_usd: 0.00575,
            },
            mr_iid: Some(42),
            pipeline_id: Some(100),
            started_at: "2026-08-01T10:00:00Z".into(),
            finished_at: "2026-08-01T10:02:05Z".into(),
            team_steps: Vec::new(),
        }
    }

    #[test]
    fn generate_html_contains_all_sections() {
        let data = make_sample_data();
        let html = generate_html(&data);

        // 所有主要 section 都存在
        assert!(html.contains("devnpc 运维报告"));
        assert!(html.contains("概览"));
        assert!(html.contains("passed"));
        assert!(html.contains("任务"));
        assert!(html.contains("修复登录页面 bug"));
        assert!(html.contains("执行轨迹"));
        assert!(html.contains("成本估算"));
        assert!(html.contains("产出"));
        assert!(html.contains("$0.005750"));
        assert!(html.contains("2 分 5 秒"));
        assert!(html.contains("LLM 调用"));
        assert!(html.contains("read_file"));
        assert!(html.contains("SOP 偏离"));
    }

    #[test]
    fn generate_html_escapes_special_chars() {
        let data = ReportData {
            task_description: "<script>alert('xss')</script>".into(),
            summary: "a & b < c > d".into(),
            ..make_sample_data()
        };
        let html = generate_html(&data);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&amp;"));
        assert!(html.contains("&lt;"));
        assert!(html.contains("&gt;"));
    }

    #[test]
    fn generate_html_shows_duration_in_readable_format() {
        let data = ReportData {
            duration_secs: 3661,
            ..make_sample_data()
        };
        let html = generate_html(&data);
        assert!(html.contains("1 时 1 分 1 秒"));
    }

    #[test]
    fn generate_html_displays_trajectory_count() {
        let data = make_sample_data();
        let html = generate_html(&data);
        assert!(html.contains("4 个事件"));
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(0), "0 秒");
        assert_eq!(format_duration(30), "30 秒");
        assert_eq!(format_duration(90), "1 分 30 秒");
        assert_eq!(format_duration(3661), "1 时 1 分 1 秒");
    }

    #[test]
    fn html_escape_handles_ampersand_first() {
        assert_eq!(html_escape("a&b"), "a&amp;b");
        assert_eq!(html_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(html_escape("\"quote\""), "&quot;quote&quot;");
    }
}