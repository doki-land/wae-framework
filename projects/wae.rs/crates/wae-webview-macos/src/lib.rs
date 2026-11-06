//! macOS WKWebView 后端占位实现。
//!
//! 真实实现（窗口 / 加载 / 消息通道 / 关闭）在后续逻辑块补齐；当前所有操作
//! 返回明确的未实现错误，不做跨平台降级。

#![deny(missing_docs)]

use wae_webview::{IpcHandler, OpenOptions, Result, WebViewBackend, WebViewError};

/// macOS WKWebView 后端。
pub struct MacosBackend {
    _private: (),
}

impl WebViewBackend for MacosBackend {
    const NAME: &'static str = "macos";

    fn open(options: OpenOptions, handler: Box<dyn IpcHandler>) -> Result<Self> {
        let _ = (options, handler);
        Err(WebViewError::Message("macOS WKWebView 后端尚未实现".into()))
    }

    fn pump(&mut self) -> bool {
        false
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn post_message(&self, json: &str) -> Result<()> {
        let _ = json;
        Err(WebViewError::Message("macOS WKWebView 后端尚未实现".into()))
    }

    fn is_alive(&self) -> bool {
        false
    }
}
