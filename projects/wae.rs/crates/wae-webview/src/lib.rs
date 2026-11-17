//! WAE WebView 后端合同：平台无关类型、后端 trait 与泛型宿主。
//!
//! 平台实现分别在 `wae-webview-win32` / `wae-webview-macos` / `wae-webview-linux`，
//! 由门面 crate `wae` 按 `cfg(target_os)` 选定一个实现并实例化 [`WebViewHost`]。
//! 本 crate 自身不依赖任何平台 crate，也不做运行时后端探测。

#![deny(missing_docs)]

use std::{fmt, sync::Arc};

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
    /// 初始宽度（逻辑像素 / DIP，96 DPI 基准）。
    ///
    /// 与 macOS / Linux 后端的点、GTK 逻辑像素一致；Windows 后端按窗口所在
    /// 监视器的有效 DPI 换算为物理像素。
    pub width: i32,
    /// 初始高度（逻辑像素 / DIP，96 DPI 基准）。
    pub height: i32,
    /// 映射到 `https://app.local/` 的本地内容目录（推荐，规避 `file://` 编码问题）。
    ///
    /// 非 Windows 后端没有虚拟主机映射，会退化为本地文件 URL；详见设计文档。
    pub content_dir: Option<String>,
    /// 未设置 `content_dir` 时直接导航的 URL。
    pub url: Option<String>,
    /// 接受拖放到窗口的本地文件，并把完整路径推送给页面。
    ///
    /// 消息形状固定为 `{"type":"fileDrop","paths":[...]}`，由宿主主动推送。
    /// Windows 已接线（`WM_DROPFILES`）；macOS / Linux 后端接收字段但本轮不接线，
    /// 补齐实现时不得更改消息形状。
    pub accept_file_drop: bool,
    /// 打开窗口但不显示，供自动化 harness 运行；默认显示。
    ///
    /// 后端无法隐藏窗口时必须返回 [`WebViewError::Unsupported`]
    /// （`Capability::HiddenWindow`），不得静默显示或静默空白。
    pub hidden: bool,
    /// 是否带原生标题栏与边框装饰；默认 `true`。
    ///
    /// 设为 `false` 时窗口不再有原生标题栏，页面自绘标题栏必须经
    /// [`WebViewHost::set_drag_regions`] 上报可拖拽矩形（客户区逻辑像素），
    /// 否则窗口无法拖动。平台无法去掉装饰时必须返回 [`WebViewError::Unsupported`]
    /// （`Capability::Decorations`），不得静默保留原生标题栏。
    pub decorations: bool,
}

/// 页面自绘标题栏的拖拽区域（客户区坐标，逻辑像素 / DIP）。
///
/// 落在区域内的命中测试按标题栏处理：可拖动窗口，双击最大化 / 还原。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragRect {
    /// 区域左缘（客户区逻辑像素）。
    pub x: i32,
    /// 区域上缘（客户区逻辑像素）。
    pub y: i32,
    /// 区域宽度（逻辑像素）。
    pub width: i32,
    /// 区域高度（逻辑像素）。
    pub height: i32,
}

impl OpenOptions {
    /// 通过本地内容目录打开。
    pub fn from_content_dir(content_dir: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            width: 1100,
            height: 720,
            content_dir: Some(content_dir.into()),
            url: None,
            accept_file_drop: false,
            hidden: false,
            decorations: true,
        }
    }

    /// 直接导航到 URL（逃生舱；优先使用 [`Self::from_content_dir`]）。
    pub fn from_url(url: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            width: 1100,
            height: 720,
            content_dir: None,
            url: Some(url.into()),
            accept_file_drop: false,
            hidden: false,
            decorations: true,
        }
    }

    /// 切换隐藏窗口运行（自动化用）。
    pub fn with_hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
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

/// 宿主 → 页面的线程安全投递句柄。
///
/// 框架把 [`IpcHandler::on_ipc`] 放在专用 IPC worker 线程执行；处理者可以把本句柄
/// clone 进其它线程，随时投递迟到回复或进度事件。投递统一由各后端 `pump()` 在
/// 拥有窗口的线程排空，因此不违反平台线程约束（macOS 主线程 / Linux GTK 初始化线程），
/// 也不会让慢业务停在窗口消息循环里冻结标题栏。
#[derive(Clone)]
pub struct IpcSink {
    deliver: Arc<dyn (Fn(String) -> crate::Result<()>) + Send + Sync>,
}

impl IpcSink {
    /// 由后端实现从自己的线程安全投递入口构造。
    pub fn new(deliver: impl Fn(String) -> crate::Result<()> + Send + Sync + 'static) -> Self {
        Self { deliver: Arc::new(deliver) }
    }

    /// 向页面投递一条字符串消息；窗口已销毁时返回结构化失败。
    pub fn post(&self, json: &str) -> crate::Result<()> {
        (self.deliver)(json.to_string())
    }
}

/// 处理 WebView 页面发来的消息。
pub trait IpcHandler: Send {
    /// 处理一条入站 IPC 消息。
    ///
    /// 本方法在框架的 IPC worker 线程执行，不在窗口消息循环线程：慢业务（HTTP、
    /// 磁盘）不会冻结窗口。快回复直接返回 [`IpcOutcome::ReplyToPage`]；迟到回复与
    /// 进度事件经 [`IpcSink`] 投递（句柄可 clone 到其它线程）。
    fn on_ipc(&mut self, message: &str, sink: &IpcSink) -> IpcOutcome;
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
    /// 恢复并前置已打开的窗口（单实例二次启动唤醒）。
    Focus,
    /// 接收拖放到窗口的本地文件并把完整路径推送给页面。
    FileDrop,
    /// 在页面执行脚本并回收 JSON 结果（自动化 harness）。
    ScriptEval,
    /// 把当前 WebView 可见内容截成 PNG 文件（自动化 harness）。
    Screenshot,
    /// 打开窗口但不显示（自动化 harness）。
    HiddenWindow,
    /// 最小化窗口（自绘标题栏用）。
    Minimize,
    /// 在最大化与还原之间切换（自绘标题栏用）。
    ToggleMaximize,
    /// 去掉原生标题栏与边框装饰（frameless 窗口）。
    Decorations,
    /// 设置页面自绘标题栏的拖拽区域（frameless 窗口用）。
    DragRegions,
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

/// 宿主层失败的结构化原因。
///
/// 每个失败必须能回答三件事：稳定错误码、失败阶段、底层错误码（HRESULT 或
/// Win32 错误码）；必要时附带已经尝试过的路径，供本地化与排障使用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFailure {
    /// 稳定错误码，例如 `webview2_runtime_missing`。
    pub code: &'static str,
    /// 失败阶段，例如 `resolve_runtime` / `create_window`。
    pub stage: &'static str,
    /// 底层 HRESULT 或 Win32 错误码。
    pub hresult: Option<i32>,
    /// 说明文本。
    pub message: String,
    /// 已经尝试过的路径或探测位置。
    pub attempted: Vec<String>,
}

/// 打开或驱动 WebView 宿主时的失败。
///
/// 平台缺少能力时必须用 [`WebViewError::Unsupported`]（携带 [`Capability`] 与
/// [`Platform`]），由调用方按结构化字段判别与本地化；不得用字符串表达平台不支持。
/// 宿主层运行时失败用 [`WebViewError::Host`]，禁止只回传一句无阶段、无错误码的文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebViewError {
    /// 当前平台不具备该能力。
    Unsupported {
        /// 缺少的能力。
        capability: Capability,
        /// 编译目标平台。
        platform: Platform,
    },
    /// 宿主层运行时失败（窗口、WebView2 运行时、环境、controller、导航等）。
    Host(HostFailure),
    /// 带说明文本的运行时失败（历史入口，新代码应使用 [`WebViewError::Host`]）。
    Message(String),
}

impl WebViewError {
    /// 构造结构化的「平台缺少能力」错误。
    pub fn unsupported(capability: Capability, platform: Platform) -> Self {
        Self::Unsupported { capability, platform }
    }

    /// 构造结构化的宿主层失败。
    pub fn host(code: &'static str, stage: &'static str, message: impl Into<String>) -> Self {
        Self::Host(HostFailure { code, stage, hresult: None, message: message.into(), attempted: Vec::new() })
    }

    /// 记录底层 HRESULT / Win32 错误码。
    pub fn with_hresult(mut self, hresult: i32) -> Self {
        if let Self::Host(failure) = &mut self {
            failure.hresult = Some(hresult);
        }
        self
    }

    /// 记录已经尝试过的路径或探测位置。
    pub fn with_attempted(mut self, attempted: impl IntoIterator<Item = impl Into<String>>) -> Self {
        if let Self::Host(failure) = &mut self {
            failure.attempted = attempted.into_iter().map(Into::into).collect();
        }
        self
    }

    /// 失败错误码；仅 [`WebViewError::Host`] 有。
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::Host(failure) => Some(failure.code),
            _ => None,
        }
    }

    /// 失败阶段；仅 [`WebViewError::Host`] 有。
    pub fn stage(&self) -> Option<&'static str> {
        match self {
            Self::Host(failure) => Some(failure.stage),
            _ => None,
        }
    }

    /// 底层 HRESULT / Win32 错误码；仅 [`WebViewError::Host`] 且已知时有。
    pub fn hresult(&self) -> Option<i32> {
        match self {
            Self::Host(failure) => failure.hresult,
            _ => None,
        }
    }

    /// 若为 [`WebViewError::Unsupported`]，返回缺少的能力。
    pub fn capability(&self) -> Option<Capability> {
        match self {
            Self::Unsupported { capability, .. } => Some(*capability),
            Self::Host(_) | Self::Message(_) => None,
        }
    }

    /// 若为 [`WebViewError::Unsupported`]，返回对应平台。
    pub fn platform(&self) -> Option<Platform> {
        match self {
            Self::Unsupported { platform, .. } => Some(*platform),
            Self::Host(_) | Self::Message(_) => None,
        }
    }
}

impl fmt::Display for WebViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { capability, platform } => {
                write!(f, "capability {capability:?} is not supported on platform {platform:?}")
            }
            Self::Host(failure) => {
                write!(f, "{} (stage {}): {}", failure.code, failure.stage, failure.message)?;
                if let Some(hresult) = failure.hresult {
                    write!(f, " [hresult=0x{hresult:08X}]")?;
                }
                if !failure.attempted.is_empty() {
                    write!(f, " [tried: {}]", failure.attempted.join(", "))?;
                }
                Ok(())
            }
            Self::Message(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for WebViewError {}

/// WebView 辅助函数的结果别名。
pub type Result<T> = std::result::Result<T, WebViewError>;

/// 自动化任务编号；由宿主分配，后端只负责回传。
pub type AutomationId = u64;

/// 自动化失败码（稳定字符串，供 harness 与 CLI 判别）。
pub mod automation_codes {
    /// 平台没有脚本执行能力。
    pub const SCRIPT_EVAL_UNSUPPORTED: &str = "script_eval_unsupported";
    /// 平台没有截图能力。
    pub const SCREENSHOT_UNSUPPORTED: &str = "screenshot_unsupported";
    /// 平台无法隐藏窗口。
    pub const HIDDEN_WINDOW_UNSUPPORTED: &str = "hidden_window_unsupported";
    /// 窗口已销毁，排队中的任务被取消。
    pub const WINDOW_CLOSED: &str = "window_closed";
    /// 页面脚本抛出异常。
    pub const SCRIPT_ERROR: &str = "script_error";
    /// 截图失败（流创建、编码或回调错误）。
    pub const CAPTURE_FAILED: &str = "capture_failed";
}

/// 自动化任务的稳定失败分类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationError {
    /// 稳定失败码，见 [`automation_codes`]。
    pub code: &'static str,
    /// 可读说明；不得包含凭证或页面外的宿主路径。
    pub message: String,
}

impl AutomationError {
    /// 构造一条失败记录。
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

/// 脚本执行结果；`value` 与 `error` 恰好一个非空。
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptOutcome {
    /// 任务编号。
    pub id: AutomationId,
    /// 页面返回的 JSON 值（`undefined` 视为 `null`）。
    pub value: Option<serde_json::Value>,
    /// 执行失败。
    pub error: Option<AutomationError>,
}

impl ScriptOutcome {
    /// 成功结果。
    pub fn success(id: AutomationId, value: serde_json::Value) -> Self {
        Self { id, value: Some(value), error: None }
    }

    /// 失败结果。
    pub fn failure(id: AutomationId, error: AutomationError) -> Self {
        Self { id, value: None, error: Some(error) }
    }
}

/// 截图结果；`path` 与 `error` 恰好一个非空。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureOutcome {
    /// 任务编号。
    pub id: AutomationId,
    /// 实际写入的 PNG 路径。
    pub path: Option<std::path::PathBuf>,
    /// 截图失败。
    pub error: Option<AutomationError>,
}

/// 单个平台的 WebView 后端实现（窗口、加载、消息通道、关闭、事件泵、自动化）。
pub trait WebViewBackend: Sized {
    /// 后端名，用于诊断与验证。
    const NAME: &'static str;

    /// 编译目标平台，用于结构化「能力缺失」错误。
    const PLATFORM: Platform;

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

    /// 恢复并前置已打开的窗口。
    ///
    /// 单实例二次启动按 `window.secondLaunch: 'focus'` 调用本方法。平台缺少该能力
    /// 时必须返回 [`WebViewError::Unsupported`]（`Capability::Focus`），不得静默成功。
    fn focus(&mut self) -> Result<()>;

    /// 设置页面自绘标题栏的拖拽矩形（frameless 窗口用）。
    ///
    /// 默认实现返回结构化「平台缺少能力」，未接线平台不得伪造成功。
    fn set_drag_regions(&mut self, _rects: &[DragRect]) -> Result<()> {
        Err(WebViewError::unsupported(Capability::DragRegions, Self::PLATFORM))
    }

    /// 发起一次页面脚本执行；结果经 [`Self::take_script_result`] 回收。
    ///
    /// 默认实现返回结构化「平台缺少能力」，未接线平台不得伪造成功。
    fn evaluate(&mut self, _id: AutomationId, _script: &str) -> Result<()> {
        Err(WebViewError::unsupported(Capability::ScriptEval, Self::PLATFORM))
    }

    /// 取走指定脚本任务的结果；尚未完成时返回 `None`。
    fn take_script_result(&mut self, _id: AutomationId) -> Option<ScriptOutcome> {
        None
    }

    /// 发起一次截图写入 `path`；结果经 [`Self::take_capture_result`] 回收。
    fn screenshot(&mut self, _id: AutomationId, _path: &std::path::Path) -> Result<()> {
        Err(WebViewError::unsupported(Capability::Screenshot, Self::PLATFORM))
    }

    /// 取走指定截图任务的结果；尚未完成时返回 `None`。
    fn take_capture_result(&mut self, _id: AutomationId) -> Option<CaptureOutcome> {
        None
    }

    /// 最小化窗口；平台缺少该能力时返回结构化 `Unsupported`。
    fn minimize(&mut self) -> Result<()> {
        Err(WebViewError::unsupported(Capability::Minimize, Self::PLATFORM))
    }

    /// 在最大化与还原之间切换；平台缺少该能力时返回结构化 `Unsupported`。
    fn toggle_maximize(&mut self) -> Result<()> {
        Err(WebViewError::unsupported(Capability::ToggleMaximize, Self::PLATFORM))
    }
}

/// 由外部驱动的 WebView 宿主；`B` 由门面按目标平台选定。
pub struct WebViewHost<B: WebViewBackend> {
    backend: B,
    next_automation_id: AutomationId,
}

impl<B: WebViewBackend> WebViewHost<B> {
    /// 创建并显示 WebView 窗口。
    pub fn open(options: OpenOptions, handler: Box<dyn IpcHandler>) -> Result<Self> {
        Ok(Self { backend: B::open(options, handler)?, next_automation_id: 1 })
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

    /// 恢复并前置已打开的窗口；窗口已关闭时返回结构化失败。
    pub fn focus(&mut self) -> Result<()> {
        self.backend.focus()
    }

    /// 发起页面脚本执行并返回任务编号；窗口已销毁时返回 `window_closed`。
    pub fn evaluate(&mut self, script: &str) -> Result<AutomationId> {
        self.ensure_alive("evaluate")?;
        let id = self.next_automation_id;
        self.next_automation_id += 1;
        self.backend.evaluate(id, script)?;
        Ok(id)
    }

    /// 取走脚本结果；未完成返回 `None`。
    pub fn poll_script_result(&mut self, id: AutomationId) -> Option<ScriptOutcome> {
        self.backend.take_script_result(id)
    }

    /// 发起截图并返回任务编号。
    pub fn screenshot(&mut self, path: &std::path::Path) -> Result<AutomationId> {
        self.ensure_alive("screenshot")?;
        let id = self.next_automation_id;
        self.next_automation_id += 1;
        self.backend.screenshot(id, path)?;
        Ok(id)
    }

    /// 取走截图结果；未完成返回 `None`。
    pub fn poll_capture_result(&mut self, id: AutomationId) -> Option<CaptureOutcome> {
        self.backend.take_capture_result(id)
    }

    /// 自动化调用前的窗口存活检查：销毁后以稳定码 `window_closed` 拒绝。
    fn ensure_alive(&self, stage: &'static str) -> Result<()> {
        if self.backend.is_alive() {
            return Ok(());
        }
        Err(WebViewError::host(automation_codes::WINDOW_CLOSED, stage, "窗口已销毁，自动化调用被拒绝"))
    }

    /// 最小化窗口。
    pub fn minimize(&mut self) -> Result<()> {
        self.backend.minimize()
    }

    /// 在最大化与还原之间切换。
    pub fn toggle_maximize(&mut self) -> Result<()> {
        self.backend.toggle_maximize()
    }

    /// 设置页面自绘标题栏的拖拽矩形（客户区逻辑像素）；frameless 窗口拖动的前提。
    pub fn set_drag_regions(&mut self, rects: &[DragRect]) -> Result<()> {
        self.ensure_alive("set_drag_regions")?;
        self.backend.set_drag_regions(rects)
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
        fn on_ipc(&mut self, _message: &str, _sink: &IpcSink) -> IpcOutcome {
            IpcOutcome::Ignored
        }
    }

    /// 未接线自动化的后端：默认方法必须给出结构化 `Unsupported`，不得伪造成功。
    struct DefaultBackend {
        alive: bool,
    }

    impl WebViewBackend for DefaultBackend {
        const NAME: &'static str = "test";
        const PLATFORM: Platform = Platform::Linux;

        fn open(_options: OpenOptions, _handler: Box<dyn IpcHandler>) -> Result<Self> {
            Ok(Self { alive: true })
        }

        fn pump(&mut self) -> bool {
            self.alive
        }

        fn close(&mut self) -> Result<()> {
            self.alive = false;
            Ok(())
        }

        fn post_message(&self, _json: &str) -> Result<()> {
            Ok(())
        }

        fn is_alive(&self) -> bool {
            self.alive
        }

        fn focus(&mut self) -> Result<()> {
            Ok(())
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
        assert!(!from_dir.accept_file_drop);
        assert!(!from_dir.hidden);

        let from_url = OpenOptions::from_url("https://example.com", "App");
        assert!(from_url.content_dir.is_none());
        assert_eq!(from_url.url.as_deref(), Some("https://example.com"));
        assert!(!from_url.accept_file_drop);
        assert!(from_url.with_hidden(true).hidden);
    }

    #[test]
    fn automation_defaults_report_structured_unsupported() {
        let mut host = WebViewHost::<DefaultBackend>::open(OpenOptions::from_url("https://example.com", "App"), Box::new(NoopHandler))
            .expect("open test backend");

        let error = host.evaluate("1 + 1").expect_err("evaluate must be unsupported");
        assert_eq!(error.capability(), Some(Capability::ScriptEval));
        assert_eq!(error.platform(), Some(Platform::Linux));

        let error = host.screenshot(std::path::Path::new("shot.png")).expect_err("screenshot must be unsupported");
        assert_eq!(error.capability(), Some(Capability::Screenshot));

        assert!(host.poll_script_result(1).is_none());
        assert!(host.poll_capture_result(1).is_none());
    }

    #[test]
    fn closed_window_rejects_new_automation_calls() {
        let mut host = WebViewHost::<DefaultBackend>::open(OpenOptions::from_url("https://example.com", "App"), Box::new(NoopHandler))
            .expect("open test backend");
        host.close().expect("close");
        let error = host.evaluate("1").expect_err("closed window must reject evaluate");
        assert_eq!(error.code(), Some(automation_codes::WINDOW_CLOSED));
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
            WebViewError::Host(failure) => panic!("unexpected host error: {failure:?}"),
            WebViewError::Message(message) => panic!("unexpected message error: {message}"),
        }
        assert!(error.to_string().contains("EventPump"));

        let runtime = WebViewError::Message("boom".into());
        assert_eq!(runtime.capability(), None);
        assert_eq!(runtime.platform(), None);
    }

    #[test]
    fn host_error_carries_code_stage_hresult_and_attempted() {
        let error = WebViewError::host("webview2_runtime_missing", "resolve_runtime", "missing runtime")
            .with_hresult(0x8000_4005u32 as i32)
            .with_attempted(["HKLM\\EdgeUpdate", "HKCU\\EdgeUpdate"]);
        assert_eq!(error.code(), Some("webview2_runtime_missing"));
        assert_eq!(error.stage(), Some("resolve_runtime"));
        assert_eq!(error.hresult(), Some(0x8000_4005u32 as i32));
        assert_eq!(error.capability(), None);
        assert_eq!(error.platform(), None);
        let text = error.to_string();
        assert!(text.contains("webview2_runtime_missing (stage resolve_runtime): missing runtime"));
        assert!(text.contains("hresult=0x80004005"));
        assert!(text.contains("tried: HKLM\\EdgeUpdate, HKCU\\EdgeUpdate"));
    }
}
