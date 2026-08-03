//! 报告推送
//!
//! 将 HTML 报告写入 .devnpc-report/ 目录,支持 Artifact / Pages / None 三种目标。

use std::path::PathBuf;

use crate::config::{ReportConfig, ReportTarget};
use crate::error::Result;

/// 推送报告,返回报告文件的绝对路径
pub async fn publish(
    html: &str,
    target: &ReportTarget,
    report_config: &ReportConfig,
) -> Result<String> {
    match target {
        ReportTarget::None => {
            tracing::info!("报告目标为 None, 不写入文件");
            // 仍写入临时目录供调试
            let report_path = get_report_path(report_config)?;
            tokio::fs::create_dir_all(report_path.parent().unwrap_or(&report_path)).await?;
            tokio::fs::write(&report_path, html).await?;
            tracing::info!(path = %report_path.display(), "报告已写入 (None 模式)");
            Ok(report_path.to_string_lossy().to_string())
        }
        ReportTarget::Artifact | ReportTarget::Pages => {
            let report_path = get_report_path(report_config)?;
            tokio::fs::create_dir_all(report_path.parent().unwrap_or(&report_path)).await?;
            tokio::fs::write(&report_path, html).await?;
            tracing::info!(
                path = %report_path.display(),
                target = ?target,
                "报告已写入"
            );
            Ok(report_path.to_string_lossy().to_string())
        }
    }
}

/// 获取报告文件的完整路径
fn get_report_path(report_config: &ReportConfig) -> Result<PathBuf> {
    let cwd = std::env::current_dir().map_err(|e| {
        tracing::error!("获取当前工作目录失败: {e}");
        crate::error::DevnpcError::Config(format!("无法获取当前目录: {e}"))
    })?;
    Ok(cwd
        .join(&report_config.output_dir)
        .join(&report_config.output_file))
}

/// 获取报告目录路径
pub fn get_report_dir(report_config: &ReportConfig) -> Result<PathBuf> {
    let cwd = std::env::current_dir().map_err(|e| {
        crate::error::DevnpcError::Config(format!("无法获取当前目录: {e}"))
    })?;
    Ok(cwd.join(&report_config.output_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tokio::sync::Mutex;

    /// 序列化 CWD 修改操作,避免并发测试互相干扰
    static CWD_LOCK: Mutex<()> = Mutex::const_new(());

    /// 在临时目录中执行异步测试,自动恢复原 CWD
    async fn run_in_temp_dir<F, Fut>(f: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let _lock = CWD_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        f().await;
        std::env::set_current_dir(original_cwd).unwrap();
    }

    #[tokio::test]
    async fn publish_artifact_creates_file() {
        let html = "<h1>Test Report</h1>";
        run_in_temp_dir(|| async {
            let cfg = ReportConfig::default();
            let path = publish(html, &ReportTarget::Artifact, &cfg).await.unwrap();
            let path_buf = Path::new(&path);
            let expected = Path::new(".devnpc-report").join("report.html");
            assert!(
                path_buf.ends_with(&expected),
                "path {path:?} should end with {expected:?}"
            );

            let content = fs::read_to_string(&path).unwrap();
            assert_eq!(content, html);
        })
        .await;
    }

    #[tokio::test]
    async fn publish_pages_creates_file() {
        let html = "<h1>Pages Report</h1>";
        run_in_temp_dir(|| async {
            let cfg = ReportConfig::default();
            let path = publish(html, &ReportTarget::Pages, &cfg).await.unwrap();
            let path_buf = Path::new(&path);
            let expected = Path::new(".devnpc-report").join("report.html");
            assert!(
                path_buf.ends_with(&expected),
                "path {path:?} should end with {expected:?}"
            );

            let content = fs::read_to_string(&path).unwrap();
            assert_eq!(content, html);
        })
        .await;
    }

    #[tokio::test]
    async fn publish_none_still_writes_file() {
        let html = "<h1>None Report</h1>";
        run_in_temp_dir(|| async {
            let cfg = ReportConfig::default();
            let path = publish(html, &ReportTarget::None, &cfg).await.unwrap();
            let path_buf = Path::new(&path);
            let expected = Path::new(".devnpc-report").join("report.html");
            assert!(
                path_buf.ends_with(&expected),
                "path {path:?} should end with {expected:?}"
            );

            let content = fs::read_to_string(&path).unwrap();
            assert_eq!(content, html);
        })
        .await;
    }

    #[test]
    fn get_report_dir_returns_correct_path() {
        let _lock = CWD_LOCK.blocking_lock();
        let dir = tempfile::tempdir().unwrap();
        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let cfg = ReportConfig::default();
        let report_dir = get_report_dir(&cfg).unwrap();
        assert!(report_dir.to_string_lossy().ends_with(".devnpc-report"));

        std::env::set_current_dir(original_cwd).unwrap();
    }
}