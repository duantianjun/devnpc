//! 环境变量读取与类型解析

use crate::config::SopMode;
use crate::error::{DevnpcError, Result};

/// 从环境变量读取字符串,缺失则返回错误
pub fn get_required(var: &str) -> Result<String> {
    std::env::var(var).map_err(|_| DevnpcError::MissingEnv { var: var.into() })
}

/// 从环境变量读取字符串,缺失返回默认值
pub fn get_or_default(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.into())
}

/// 读取可选字符串环境变量 (缺失返回 None)
pub fn get_optional(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

/// 读取并解析为 u32,缺失返回 None,解析失败返回错误
pub fn get_u32(var: &str) -> Result<Option<u32>> {
    match std::env::var(var) {
        Ok(s) => s
            .parse::<u32>()
            .map(Some)
            .map_err(|_| DevnpcError::Config(format!("环境变量 {var} 不是有效 u32: {s}"))),
        Err(_) => Ok(None),
    }
}

/// 读取并解析为 u8,缺失返回 None,解析失败返回错误
pub fn get_u8(var: &str) -> Result<Option<u8>> {
    match std::env::var(var) {
        Ok(s) => s
            .parse::<u8>()
            .map(Some)
            .map_err(|_| DevnpcError::Config(format!("环境变量 {var} 不是有效 u8: {s}"))),
        Err(_) => Ok(None),
    }
}

/// 读取并解析 SopMode,缺失返回 None,非法值返回错误
pub fn get_sop_mode(var: &str) -> Result<Option<SopMode>> {
    match std::env::var(var) {
        Ok(s) => match s.as_str() {
            "soft" => Ok(Some(SopMode::Soft)),
            "strict" => Ok(Some(SopMode::Strict)),
            _ => Err(DevnpcError::Config(format!(
                "环境变量 {var} 必须是 soft|strict,实际: {s}"
            ))),
        },
        Err(_) => Ok(None),
    }
}

/// 读取并解析 ReportTarget,缺失返回 None,非法值返回错误
pub fn get_report_target(var: &str) -> Result<Option<crate::config::ReportTarget>> {
    use crate::config::ReportTarget;
    match std::env::var(var) {
        Ok(s) => match s.as_str() {
            "artifact" => Ok(Some(ReportTarget::Artifact)),
            "pages" => Ok(Some(ReportTarget::Pages)),
            "none" => Ok(Some(ReportTarget::None)),
            _ => Err(DevnpcError::Config(format!(
                "环境变量 {var} 必须是 artifact|pages|none,实际: {s}"
            ))),
        },
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 注意: 环境变量测试用唯一前缀 DEVNPC_TEST_ENV_ 避免并行污染
    const PREFIX: &str = "DEVNPC_TEST_ENV_";

    fn set_var(key: &str, val: &str) {
        std::env::set_var(key, val);
    }

    fn remove_var(key: &str) {
        std::env::remove_var(key);
    }

    #[test]
    fn get_required_returns_value_when_set() {
        let key = format!("{PREFIX}REQUIRED_SET");
        set_var(&key, "abc");
        assert_eq!(get_required(&key).unwrap(), "abc");
        remove_var(&key);
    }

    #[test]
    fn get_required_returns_error_when_missing() {
        let key = format!("{PREFIX}REQUIRED_MISSING");
        remove_var(&key);
        let err = get_required(&key).unwrap_err();
        assert!(matches!(err, DevnpcError::MissingEnv { .. }));
    }

    #[test]
    fn get_or_default_returns_default_when_missing() {
        let key = format!("{PREFIX}DEFAULT_MISSING");
        remove_var(&key);
        assert_eq!(get_or_default(&key, "fallback"), "fallback");
    }

    #[test]
    fn get_u32_parses_valid() {
        let key = format!("{PREFIX}U32_VALID");
        set_var(&key, "42");
        assert_eq!(get_u32(&key).unwrap(), Some(42));
        remove_var(&key);
    }

    #[test]
    fn get_u32_returns_none_when_missing() {
        let key = format!("{PREFIX}U32_MISSING");
        remove_var(&key);
        assert_eq!(get_u32(&key).unwrap(), None);
    }

    #[test]
    fn get_u32_returns_error_on_invalid() {
        let key = format!("{PREFIX}U32_INVALID");
        set_var(&key, "not-a-number");
        assert!(get_u32(&key).is_err());
        remove_var(&key);
    }

    #[test]
    fn get_u8_parses_valid() {
        let key = format!("{PREFIX}U8_VALID");
        set_var(&key, "5");
        assert_eq!(get_u8(&key).unwrap(), Some(5));
        remove_var(&key);
    }

    #[test]
    fn get_sop_mode_parses_soft() {
        let key = format!("{PREFIX}SOP_SOFT");
        set_var(&key, "soft");
        assert_eq!(get_sop_mode(&key).unwrap(), Some(SopMode::Soft));
        remove_var(&key);
    }

    #[test]
    fn get_sop_mode_parses_strict() {
        let key = format!("{PREFIX}SOP_STRICT");
        set_var(&key, "strict");
        assert_eq!(get_sop_mode(&key).unwrap(), Some(SopMode::Strict));
        remove_var(&key);
    }

    #[test]
    fn get_sop_mode_returns_error_on_invalid() {
        let key = format!("{PREFIX}SOP_INVALID");
        set_var(&key, "loud");
        assert!(get_sop_mode(&key).is_err());
        remove_var(&key);
    }

    #[test]
    fn get_report_target_parses_pages() {
        let key = format!("{PREFIX}RT_PAGES");
        set_var(&key, "pages");
        assert_eq!(
            get_report_target(&key).unwrap(),
            Some(crate::config::ReportTarget::Pages)
        );
        remove_var(&key);
    }
}
