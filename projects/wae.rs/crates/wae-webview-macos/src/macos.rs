//! macOS WKWebView 实现：`NSWindow` + `WKWebView`，消息通道为
//! `window.webkit.messageHandlers.wae`，宿主回投调用 `window.__waeReceive`。

use std::sync::mpsc;

use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{NSObject, NSObjectProtocol},
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSAutoresizingMaskOptions, NSBackingStoreType, NSEventMask, NSView, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKScriptMessage, WKScriptMessageHandler, WKUserContentController, WKWebView, WKWebViewConfiguration};
use wae_webview::{
    IpcHandler, IpcOutcome, IpcSink, MESSAGE_HANDLER_NAME, OpenOptions, Result, WebViewBackend, WebViewError, page_receive_script,
};

/// worker 线程排给主线程的出站动作；`pump()` 在拥有窗口的线程排空。
enum Outbound {
    /// 向页面投递一条消息。
    Post(String),
    /// 关闭窗口。
    Close,
}

struct IpcIvars {
    inbox_tx: mpsc::Sender<String>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WaeIpcHandler"]
    #[ivars = IpcIvars]
    struct IpcHandlerObject;

    impl IpcHandlerObject {}

    unsafe impl NSObjectProtocol for IpcHandlerObject {}

    unsafe impl WKScriptMessageHandler for IpcHandlerObject {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            let body = unsafe { message.body() };
            let Ok(text) = body.downcast::<NSString>() else {
                return;
            };
            // 入站消息直接交给 IPC worker 线程；主线程消息循环不再执行处理者。
            let _ = self.ivars().inbox_tx.send(text.to_string());
        }
    }
);

impl IpcHandlerObject {
    fn new(mtm: MainThreadMarker, inbox_tx: mpsc::Sender<String>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(IpcIvars { inbox_tx });
        unsafe { msg_send![super(this), init] }
    }
}

/// macOS WKWebView 后端。
pub struct MacosBackend {
    app: Retained<NSApplication>,
    window: Retained<NSWindow>,
    webview: Retained<WKWebView>,
    /// 仅用于保活 ObjC 侧消息处理器（入站消息经它的 `inbox_tx` 直达 worker 线程）。
    #[allow(dead_code)]
    ipc: Retained<IpcHandlerObject>,
    outbox_rx: mpsc::Receiver<Outbound>,
    alive: bool,
}

impl MacosBackend {
    /// 在主线程排空 IPC worker 与 `IpcSink` 投递的出站动作。
    fn drain_outbox(&mut self) {
        loop {
            match self.outbox_rx.try_recv() {
                Ok(Outbound::Post(json)) => {
                    let _ = self.post_message(&json);
                }
                Ok(Outbound::Close) => {
                    let _ = self.close();
                    return;
                }
                Err(_) => return,
            }
        }
    }
}

impl WebViewBackend for MacosBackend {
    const NAME: &'static str = "macos";

    fn open(options: OpenOptions, handler: Box<dyn IpcHandler>) -> Result<Self> {
        let Some(mtm) = MainThreadMarker::new()
        else {
            return Err(WebViewError::Message("macOS WebView 必须在主线程打开".into()));
        };
        if options.content_dir.is_none() && options.url.is_none() {
            return Err(WebViewError::Message("打开 WebView 需要 content_dir 或 url".into()));
        }

        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        // `activateIgnoringOtherApps:` 在 macOS 14 起标记弃用，但仍是旧系统上唯一的激活入口。
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);

        let frame = NSRect::new(NSPoint::new(120.0, 120.0), NSSize::new(options.width as f64, options.height as f64));
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, style, NSBackingStoreType::Buffered, false)
        };
        window.setTitle(&NSString::from_str(&options.title));
        unsafe { window.setReleasedWhenClosed(false) };
        window.center();

        let config = unsafe { WKWebViewConfiguration::new(mtm) };
        let controller = unsafe { config.userContentController() };
        let (inbox_tx, inbox_rx) = mpsc::channel::<String>();
        let (outbox_tx, outbox_rx) = mpsc::channel::<Outbound>();
        let ipc = IpcHandlerObject::new(mtm, inbox_tx);
        unsafe {
            controller.addScriptMessageHandler_name(objc2::runtime::ProtocolObject::from_ref(&*ipc), &NSString::from_str(MESSAGE_HANDLER_NAME));
        }

        let content_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(options.width as f64, options.height as f64));
        let webview = unsafe { WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), content_frame, &config) };
        webview.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        let view: &NSView = &webview;
        window.setContentView(Some(view));
        window.makeKeyAndOrderFront(None);

        if let Some(dir) = &options.content_dir {
            let dir = std::fs::canonicalize(dir).map_err(|error| WebViewError::Message(format!("content_dir 无法解析: {error}")))?;
            let index = dir.join("index.html");
            if !index.is_file() {
                return Err(WebViewError::Message(format!("content_dir 缺少 index.html: {}", index.display())));
            }
            let dir_url = NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()));
            let file_url = NSURL::fileURLWithPath(&NSString::from_str(&index.to_string_lossy()));
            unsafe { webview.loadFileURL_allowingReadAccessToURL(&file_url, &dir_url) };
        }
        else if let Some(url) = &options.url {
            let parsed = NSURL::URLWithString(&NSString::from_str(url)).ok_or_else(|| WebViewError::Message(format!("无效 URL: {url}")))?;
            let request = NSURLRequest::requestWithURL(&parsed);
            unsafe { webview.loadRequest(&request) };
        }

        // IPC worker 线程：处理者在专用线程执行，主线程消息循环不再被业务阻塞；
        // 出站动作经 outbox 回到主线程，由 `pump()` 排空投递。
        let sink = {
            let tx = outbox_tx.clone();
            IpcSink::new(move |json| tx.send(Outbound::Post(json)).map_err(|_| WebViewError::Message("macOS WebView 窗口已关闭".into())))
        };
        let push_close = {
            let tx = outbox_tx;
            move || tx.send(Outbound::Close).is_ok()
        };
        std::thread::Builder::new()
            .name("wae-ipc".to_string())
            .spawn(move || {
                let mut handler = handler;
                for message in inbox_rx.iter() {
                    match handler.on_ipc(&message, &sink) {
                        IpcOutcome::ReplyToPage(json) => {
                            let _ = sink.post(&json);
                        }
                        IpcOutcome::CloseWindow => {
                            let _ = push_close();
                        }
                        IpcOutcome::Ignored => {}
                    }
                }
            })
            .map_err(|error| WebViewError::Message(format!("IPC worker 线程启动失败: {error}")))?;

        Ok(Self { app, window, webview, ipc, outbox_rx, alive: true })
    }

    fn pump(&mut self) -> bool {
        if !self.alive {
            return false;
        }

        while let Some(event) = self.app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::Any,
            Some(&NSDate::distantPast()),
            unsafe { NSDefaultRunLoopMode },
            true,
        ) {
            self.app.sendEvent(&event);
        }

        self.drain_outbox();
        if !self.alive || !self.window.isVisible() {
            self.alive = false;
            return false;
        }
        true
    }

    fn close(&mut self) -> Result<()> {
        if self.alive {
            self.alive = false;
            self.window.close();
        }
        Ok(())
    }

    fn post_message(&self, json: &str) -> Result<()> {
        if !self.alive {
            return Err(WebViewError::Message("macOS WebView 窗口已关闭".into()));
        }
        let script = page_receive_script(json);
        unsafe { self.webview.evaluateJavaScript_completionHandler(&NSString::from_str(&script), None) };
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.alive
    }

    fn focus(&mut self) -> Result<()> {
        if !self.alive {
            return Err(WebViewError::host("window_not_alive", "focus", "窗口已关闭，无法唤醒"));
        }
        self.window.makeKeyAndOrderFront(None);
        #[allow(deprecated)]
        self.app.activateIgnoringOtherApps(true);
        Ok(())
    }
}
