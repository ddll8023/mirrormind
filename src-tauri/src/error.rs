//! 定义壳层可公开的结构化错误，不记录用户输入正文。

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
}

impl AppError {
    /// 构造可公开的错误；调用者只传入脱敏消息，不传入记录正文或 SQL 参数。
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for AppError {
    /// 将公开错误码与消息用于系统诊断，不额外展开底层错误对象。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}
