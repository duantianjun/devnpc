//! 环境变量读取 (P1 完整实现)

use crate::error::{DevnpcError, Result};

/// 从环境变量读取,缺失则返回错误
pub fn get_required(var: &str) -> Result<String> {
    std::env::var(var).map_err(|_| DevnpcError::MissingEnv { var: var.into() })
}

/// 从环境变量读取,缺失返回默认值
pub fn get_or_default(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.into())
}
