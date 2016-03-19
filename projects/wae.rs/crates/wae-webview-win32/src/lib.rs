//! 自研 Win32 + WebView2 宿主，提供非阻塞的 [`WebViewHost::pump`]。

#![deny(missing_docs)]

mod error;
#[cfg(windows)]
mod win32;

pub use error::{Result, WebViewError};

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
    pub content_dir: Option<String>,
    /// 未设置 `content_dir` 时直接导航的 URL。
    pub url: Option<String>,
}

impl OpenOptions {
    /// 通过虚拟主机 `https://app.local/` 提供本地内容目录。
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

/// 由外部驱动的 WebView 宿主。用 [`WebViewHost::open`] 创建，用 [`WebViewHost::pump`] 抽消息。
pub struct WebViewHost {
    #[cfg(windows)]
    inner: win32::HostInner,
    #[cfg(not(windows))]
    _private: (),
}

impl WebViewHost {
    /// 创建并显示 WebView 窗口。
    pub fn open(options: OpenOptions, handler: Box<dyn IpcHandler>) -> Result<Self> {
        #[cfg(windows)]
        {
            Ok(Self { inner: win32::HostInner::open(options, handler)? })
        }
        #[cfg(not(windows))]
        {
            let _ = (options, handler);
            Err(WebViewError::Unsupported("wae-webview-win32 GUI 仅在 Windows 可用"))
        }
    }

    /// 处理挂起的 Win32 / WebView2 消息。
    ///
    /// 窗口已关闭时返回 `false`。
    pub fn pump(&mut self) -> bool {
        #[cfg(windows)]
        {
            self.inner.pump()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// 请求关闭宿主窗口。
    pub fn close(&mut self) -> Result<()> {
        #[cfg(windows)]
        {
            self.inner.close()
        }
        #[cfg(not(windows))]
        {
            Ok(())
        }
    }

    /// 向页面投递一条字符串消息。
    pub fn post_message(&self, json: &str) -> Result<()> {
        #[cfg(windows)]
        {
            self.inner.post_message(json)
        }
        #[cfg(not(windows))]
        {
            let _ = json;
            Err(WebViewError::Unsupported("wae-webview-win32 GUI 仅在 Windows 可用"))
        }
    }

    /// 窗口是否仍处于打开状态。
    pub fn is_alive(&self) -> bool {
        #[cfg(windows)]
        {
            self.inner.is_alive()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
}
