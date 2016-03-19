//! Win32 WebView 宿主的错误类型。

use std::fmt;

/// 打开或驱动 WebView 宿主时的失败。
#[derive(Debug)]
pub enum WebViewError {
    /// 当前目标平台不支持该能力。
    Unsupported(&'static str),
    /// 带说明文本的运行时失败。
    Message(String),
}

impl fmt::Display for WebViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(msg) => f.write_str(msg),
            Self::Message(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for WebViewError {}

/// WebView 辅助函数的结果别名。
pub type Result<T> = std::result::Result<T, WebViewError>;
