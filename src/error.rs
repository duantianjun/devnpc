//! 统一错误类型

use thiserror::Error;

#[derive(Error, Debug)]
pub enum DevnpcError {
    #[error("占位错误 - P0 骨架")]
    Placeholder,
}

pub type Result<T> = std::result::Result<T, DevnpcError>;
