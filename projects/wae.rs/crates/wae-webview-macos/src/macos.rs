//! macOS WKWebView 实现：`NSWindow` + `WKWebView`，消息通道为
//! `window.webkit.messageHandlers.wae`，宿主回投调用 `window.__waeReceive`。

use std::{collections::VecDeque, sync::Mutex};

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
use wae_webview::{IpcHandler, IpcOutcome, MESSAGE_HANDLER_NAME, OpenOptions, Result, WebViewBackend, WebViewError, page_receive_script};

struct IpcIvars {
    messages: Mutex<VecDeque<String>>,
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
            if let Ok(mut queue) = self.ivars().messages.lock() {
                queue.push_back(text.to_string());
            }
        }
    }
);

impl IpcHandlerObject {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(IpcIvars { messages: Mutex::new(VecDeque::new()) });
        unsafe { msg_send![super(this), init] }
    }

    fn take_message(&self) -> Option<String> {
        self.ivars().messages.lock().ok().and_then(|mut queue| queue.pop_front())
    }
}

/// macOS WKWebView 后端。
pub struct MacosBackend {
    app: Retained<NSApplication>,
    window: Retained<NSWindow>,
    webview: Retained<WKWebView>,
    ipc: Retained<IpcHandlerObject>,
    handler: Box<dyn IpcHandler>,
    alive: bool,
}

impl MacosBackend {
    fn dispatch_pending(&mut self) {
        while let Some(message) = self.ipc.take_message() {
            match self.handler.on_ipc(&message) {
                IpcOutcome::ReplyToPage(json) => {
                    let _ = self.post_message(&json);
                }
                IpcOutcome::CloseWindow => {
                    let _ = self.close();
                    return;
                }
                IpcOutcome::Ignored => {}
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
        let ipc = IpcHandlerObject::new(mtm);
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

        Ok(Self { app, window, webview, ipc, handler, alive: true })
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

        self.dispatch_pending();
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
}
