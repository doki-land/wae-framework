//! WAE WebView 后端合同：平台无关类型、后端 trait 与泛型宿主。
//!
//! 平台实现分别在 `wae-webview-win32` / `wae-webview-macos` / `wae-webview-linux`，
//! 由门面 crate `wae` 按 `cfg(target_os)` 选定一个实现并实例化 [`WebViewHost`]。
//! 本 crate 自身不依赖任何平台 crate，也不做运行时后端探测。

#![deny(missing_docs)]

use std::fmt;

/// macOS / Linux WebKit 后端的消息处理器名：页面侧为
/// `window.webkit.messageHandlers.wae`。
pub const MESSAGE_HANDLER_NAME: &str = "wae";

/// 生成宿主 → 页面的回投脚本，调用页面注册的 `window.__waeReceive(text)`。
///
/// Windows 走 WebView2 原生通道，不使用该脚本；macOS / Linux 的 WebKit
/// 没有等价的反向通道，只能通过 `evaluateJavaScript` 调用页面函数。
pub fn page_receive_script(text: &str) -> String {
    format!("if (window.__waeReceive) window.__waeReceive({});", js_string_literal(text))
}

/// 把任意字符串转成可安全嵌入 JavaScript 的字符串字面量（含 U+2028/U+2029）。
fn js_string_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

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

/// WebView 能力面；用于结构化表达「某平台缺少某项能力」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// 创建/显示原生窗口。
    Window,
    /// `content_dir` 或 `url` 内容加载。
    ContentLoad,
    /// 宿主 ↔ 页面消息通道。
    MessageChannel,
    /// 外部驱动的非阻塞事件泵。
    EventPump,
    /// 关闭窗口。
    Close,
}

/// 编译目标平台。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    /// Windows。
    Windows,
    /// macOS。
    Macos,
    /// Linux。
    Linux,
}

/// 打开或驱动 WebView 宿主时的失败。
///
/// 平台缺少能力时必须用 [`WebViewError::Unsupported`]（携带 [`Capability`] 与
/// [`Platform`]），由调用方按结构化字段判别与本地化；不得用字符串表达平台不支持。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebViewError {
    /// 当前平台不具备该能力。
    Unsupported {
        /// 缺少的能力。
        capability: Capability,
        /// 编译目标平台。
        platform: Platform,
    },
    /// 带说明文本的运行时失败。
    Message(String),
}

impl WebViewError {
    /// 构造结构化的「平台缺少能力」错误。
    pub fn unsupported(capability: Capability, platform: Platform) -> Self {
        Self::Unsupported { capability, platform }
    }

    /// 若为 [`WebViewError::Unsupported`]，返回缺少的能力。
    pub fn capability(&self) -> Option<Capability> {
        match self {
            Self::Unsupported { capability, .. } => Some(*capability),
            Self::Message(_) => None,
        }
    }

    /// 若为 [`WebViewError::Unsupported`]，返回对应平台。
    pub fn platform(&self) -> Option<Platform> {
        match self {
            Self::Unsupported { platform, .. } => Some(*platform),
            Self::Message(_) => None,
        }
    }
}

impl fmt::Display for WebViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { capability, platform } => {
                write!(f, "capability {capability:?} is not supported on platform {platform:?}")
            }
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

    #[test]
    fn page_receive_script_escapes_js_metacharacters() {
        assert_eq!(page_receive_script("a\"b"), "if (window.__waeReceive) window.__waeReceive(\"a\\\"b\");");
        assert_eq!(page_receive_script("line\nline"), "if (window.__waeReceive) window.__waeReceive(\"line\\nline\");");
        assert_eq!(page_receive_script("a\\b"), "if (window.__waeReceive) window.__waeReceive(\"a\\\\b\");");
        assert!(page_receive_script("\u{2028}").contains("\\u2028"));
        assert!(page_receive_script("\u{1}").contains("\\u0001"));
    }

    #[test]
    fn unsupported_error_carries_capability_and_platform() {
        let error = WebViewError::unsupported(Capability::EventPump, Platform::Linux);
        assert_eq!(error.capability(), Some(Capability::EventPump));
        assert_eq!(error.platform(), Some(Platform::Linux));
        match &error {
            WebViewError::Unsupported { capability, platform } => {
                assert_eq!(*capability, Capability::EventPump);
                assert_eq!(*platform, Platform::Linux);
            }
            WebViewError::Message(message) => panic!("unexpected message error: {message}"),
        }
        assert!(error.to_string().contains("EventPump"));

        let runtime = WebViewError::Message("boom".into());
        assert_eq!(runtime.capability(), None);
        assert_eq!(runtime.platform(), None);
    }
}
