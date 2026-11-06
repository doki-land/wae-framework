//! WAE WebView 后端合同：平台无关类型、后端 trait 与泛型宿主。
//!
//! 平台实现分别在 `wae-webview-win32` / `wae-webview-macos` / `wae-webview-linux`，
//! 由门面 crate `wae` 按 `cfg(target_os)` 选定一个实现并实例化 [`WebViewHost`]。
//! 本 crate 自身不依赖任何平台 crate，也不做运行时后端探测。

#![deny(missing_docs)]

use std::fmt;

/// 打开原生 WebView 窗口的选项。
#[derive(Debug, Clone)]
pub struct OpenOptions {
    /// 窗口标题。
    pub title: String,
    /// 初始宽度（像素）。
    pub width: i32,
    /// 初始高度（像素）。
    pub height: i32,
    /// 映射到 `https://app.local/` 的本地内容目录（推荐，规避 `file://` 编码问题）。
    ///
    /// 非 Windows 后端没有虚拟主机映射，会退化为本地文件 URL；详见设计文档。
    pub content_dir: Option<String>,
    /// 未设置 `content_dir` 时直接导航的 URL。
    pub url: Option<String>,
}

impl OpenOptions {
    /// 通过本地内容目录打开。
    pub fn from_content_dir(content_dir: impl Into<String>, title: impl Into<String>) -> Self {
        Self { title: title.into(), width: 1100, height: 720, content_dir: Some(content_dir.into()), url: None }
    }

    /// 直接导航到 URL（逃生舱；优先使用 [`Self::from_content_dir`]）。
    pub fn from_url(url: impl Into<String>, title: impl Into<String>) -> Self {
        Self { title: title.into(), width: 1100, height: 720, content_dir: None, url: Some(url.into()) }
    }
}

/// 页面向宿主发送 IPC 后的处理结果。
#[derive(Debug, Clone)]
pub enum IpcOutcome {
    /// 将文本/JSON 回传给页面。
    ReplyToPage(String),
    /// 关闭宿主窗口。
    CloseWindow,
    /// 忽略该消息。
    Ignored,
}

/// 处理 WebView 页面发来的消息。
pub trait IpcHandler: Send {
    /// 处理一条入站 IPC 消息。
    fn on_ipc(&mut self, message: &str) -> IpcOutcome;
}

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

/// 单个平台的 WebView 后端实现（窗口、加载、消息通道、关闭、事件泵）。
pub trait WebViewBackend: Sized {
    /// 后端名，用于诊断与验证。
    const NAME: &'static str;

    /// 创建并显示 WebView 窗口。
    fn open(options: OpenOptions, handler: Box<dyn IpcHandler>) -> Result<Self>;

    /// 处理挂起的平台消息；窗口已关闭时返回 `false`。
    fn pump(&mut self) -> bool;

    /// 请求关闭宿主窗口。
    fn close(&mut self) -> Result<()>;

    /// 向页面投递一条字符串消息。
    fn post_message(&self, json: &str) -> Result<()>;

    /// 窗口是否仍处于打开状态。
    fn is_alive(&self) -> bool;
}

/// 由外部驱动的 WebView 宿主；`B` 由门面按目标平台选定。
pub struct WebViewHost<B: WebViewBackend> {
    backend: B,
}

impl<B: WebViewBackend> WebViewHost<B> {
    /// 创建并显示 WebView 窗口。
    pub fn open(options: OpenOptions, handler: Box<dyn IpcHandler>) -> Result<Self> {
        Ok(Self { backend: B::open(options, handler)? })
    }

    /// 处理挂起的平台消息；窗口已关闭时返回 `false`。
    pub fn pump(&mut self) -> bool {
        self.backend.pump()
    }

    /// 请求关闭宿主窗口。
    pub fn close(&mut self) -> Result<()> {
        self.backend.close()
    }

    /// 向页面投递一条字符串消息。
    pub fn post_message(&self, json: &str) -> Result<()> {
        self.backend.post_message(json)
    }

    /// 窗口是否仍处于打开状态。
    pub fn is_alive(&self) -> bool {
        self.backend.is_alive()
    }

    /// 当前后端名（例如 `win32` / `macos` / `linux`）。
    pub fn backend_name(&self) -> &'static str {
        B::NAME
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopHandler;

    impl IpcHandler for NoopHandler {
        fn on_ipc(&mut self, _message: &str) -> IpcOutcome {
            IpcOutcome::Ignored
        }
    }

    #[test]
    fn open_options_defaults_are_stable() {
        let from_dir = OpenOptions::from_content_dir("dist", "App");
        assert_eq!(from_dir.title, "App");
        assert_eq!(from_dir.width, 1100);
        assert_eq!(from_dir.height, 720);
        assert_eq!(from_dir.content_dir.as_deref(), Some("dist"));
        assert!(from_dir.url.is_none());

        let from_url = OpenOptions::from_url("https://example.com", "App");
        assert!(from_url.content_dir.is_none());
        assert_eq!(from_url.url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn handler_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Box<dyn IpcHandler>>();
        let _ = NoopHandler;
    }
}
