//! Linux WebKitGTK 4.1 + GTK3 实现：`GtkWindow` + `WebKitWebView`，消息通道为
//! `window.webkit.messageHandlers.wae`，宿主回投调用 `window.__waeReceive`。
//!
//! GTK / WebKit 符号在运行时通过 `libloading` 解析，缺少运行库或符号时返回明确错误。

use std::{
    collections::VecDeque,
    ffi::{CStr, CString, c_char, c_int, c_ulong, c_void},
    path::Path,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use libloading::Library;
use wae_webview::{IpcHandler, IpcOutcome, MESSAGE_HANDLER_NAME, OpenOptions, Result, WebViewBackend, WebViewError, page_receive_script};

type GtkWidget = c_void;
type WebKitWebView = c_void;
type WebKitUserContentManager = c_void;
type WebKitJavascriptResult = c_void;
type JscValue = c_void;
type GBool = c_int;
type GULong = c_ulong;
type GAsyncCallback = Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void)>;

/// GTK_WINDOW_TOPLEVEL
const GTK_WINDOW_TOPLEVEL: c_int = 0;
/// GTK_WIN_POS_CENTER
const GTK_WIN_POS_CENTER: c_int = 1;

#[derive(Clone, Copy)]
struct GtkApi {
    gtk_init_check: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char) -> GBool,
    gtk_window_new: unsafe extern "C" fn(c_int) -> *mut GtkWidget,
    gtk_window_set_title: unsafe extern "C" fn(*mut GtkWidget, *const c_char),
    gtk_window_set_default_size: unsafe extern "C" fn(*mut GtkWidget, c_int, c_int),
    gtk_window_set_position: unsafe extern "C" fn(*mut GtkWidget, c_int),
    gtk_container_add: unsafe extern "C" fn(*mut GtkWidget, *mut GtkWidget),
    gtk_widget_show_all: unsafe extern "C" fn(*mut GtkWidget),
    gtk_widget_destroy: unsafe extern "C" fn(*mut GtkWidget),
    gtk_window_present: unsafe extern "C" fn(*mut GtkWidget),
    gtk_events_pending: unsafe extern "C" fn() -> GBool,
    gtk_main_iteration_do: unsafe extern "C" fn(GBool) -> GBool,
    g_signal_connect_data:
        unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void, *mut c_void, Option<unsafe extern "C" fn(*mut c_void)>, c_int) -> GULong,
    g_free: unsafe extern "C" fn(*mut c_void),
    webkit_web_view_new: unsafe extern "C" fn() -> *mut WebKitWebView,
    webkit_web_view_get_user_content_manager: unsafe extern "C" fn(*mut WebKitWebView) -> *mut WebKitUserContentManager,
    webkit_user_content_manager_register_script_message_handler: unsafe extern "C" fn(*mut WebKitUserContentManager, *const c_char) -> GBool,
    webkit_web_view_load_uri: unsafe extern "C" fn(*mut WebKitWebView, *const c_char),
    webkit_web_view_evaluate_javascript:
        unsafe extern "C" fn(*mut WebKitWebView, *const c_char, isize, *const c_char, *const c_char, *mut c_void, GAsyncCallback, *mut c_void),
    webkit_javascript_result_get_js_value: unsafe extern "C" fn(*mut WebKitJavascriptResult) -> *mut JscValue,
    webkit_javascript_result_unref: unsafe extern "C" fn(*mut WebKitJavascriptResult),
    jsc_value_to_string: unsafe extern "C" fn(*mut JscValue) -> *mut c_char,
}

struct Libs {
    api: GtkApi,
    // 先于 `api` 声明，保证函数指针存续期间库句柄不先被卸载。
    _gtk: Library,
    _webkit: Library,
    _jsc: Library,
}

impl Libs {
    fn load() -> Result<Self> {
        let gtk = open_first(&["libgtk-3.so.0", "libgtk-3.so"])?;
        let webkit = open_first(&["libwebkit2gtk-4.1.so.0", "libwebkit2gtk-4.1.so"])?;
        let jsc = open_first(&["libjavascriptcoregtk-4.1.so.0", "libjavascriptcoregtk-4.1.so"])?;

        // SAFETY: 句柄在本结构存活期间保持打开；符号签名按 GTK3 / WebKitGTK 4.1 C 头声明。
        let api = unsafe {
            GtkApi {
                gtk_init_check: symbol(&gtk, "gtk_init_check")?,
                gtk_window_new: symbol(&gtk, "gtk_window_new")?,
                gtk_window_set_title: symbol(&gtk, "gtk_window_set_title")?,
                gtk_window_set_default_size: symbol(&gtk, "gtk_window_set_default_size")?,
                gtk_window_set_position: symbol(&gtk, "gtk_window_set_position")?,
                gtk_container_add: symbol(&gtk, "gtk_container_add")?,
                gtk_widget_show_all: symbol(&gtk, "gtk_widget_show_all")?,
                gtk_widget_destroy: symbol(&gtk, "gtk_widget_destroy")?,
                gtk_window_present: symbol(&gtk, "gtk_window_present")?,
                gtk_events_pending: symbol(&gtk, "gtk_events_pending")?,
                gtk_main_iteration_do: symbol(&gtk, "gtk_main_iteration_do")?,
                g_signal_connect_data: symbol(&gtk, "g_signal_connect_data")?,
                g_free: symbol(&gtk, "g_free")?,
                webkit_web_view_new: symbol(&webkit, "webkit_web_view_new")?,
                webkit_web_view_get_user_content_manager: symbol(&webkit, "webkit_web_view_get_user_content_manager")?,
                webkit_user_content_manager_register_script_message_handler: symbol(
                    &webkit,
                    "webkit_user_content_manager_register_script_message_handler",
                )?,
                webkit_web_view_load_uri: symbol(&webkit, "webkit_web_view_load_uri")?,
                webkit_web_view_evaluate_javascript: symbol(&webkit, "webkit_web_view_evaluate_javascript")?,
                webkit_javascript_result_get_js_value: symbol(&webkit, "webkit_javascript_result_get_js_value")?,
                webkit_javascript_result_unref: symbol(&webkit, "webkit_javascript_result_unref")?,
                jsc_value_to_string: symbol(&jsc, "jsc_value_to_string")?,
            }
        };

        Ok(Self { api, _gtk: gtk, _webkit: webkit, _jsc: jsc })
    }
}

fn open_first(names: &[&'static str]) -> Result<Library> {
    let mut failures = Vec::new();
    for name in names {
        // SAFETY: 只加载本机动态库，路径来自固定白名单。
        match unsafe { Library::new(*name) } {
            Ok(library) => return Ok(library),
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    let message = format!("缺少系统运行库（{}）", failures.join("; "));
    Err(WebViewError::host("webkit_dependency_missing", "load_libraries", message).with_attempted(failures))
}

/// # Safety
///
/// 调用方保证 `T` 与库中导出符号的真实签名一致；库句柄必须比返回的函数指针存活更久。
unsafe fn symbol<T: Copy>(library: &Library, name: &str) -> Result<T> {
    let value = unsafe { library.get::<T>(name.as_bytes()) }
        .map_err(|error| WebViewError::host("webkit_symbol_missing", "resolve_symbols", format!("缺少符号 {name}: {error}")))?;
    Ok(*value)
}

struct Shared {
    queue: Mutex<VecDeque<String>>,
    alive: AtomicBool,
    result_get_value: unsafe extern "C" fn(*mut WebKitJavascriptResult) -> *mut JscValue,
    value_to_string: unsafe extern "C" fn(*mut JscValue) -> *mut c_char,
    result_unref: unsafe extern "C" fn(*mut WebKitJavascriptResult),
    g_free: unsafe extern "C" fn(*mut c_void),
}

unsafe extern "C" fn on_script_message(_manager: *mut c_void, result: *mut c_void, data: *mut c_void) {
    if result.is_null() || data.is_null() {
        return;
    }
    let shared = unsafe { &*(data as *const Shared) };
    let value = unsafe { (shared.result_get_value)(result) };
    if value.is_null() {
        return;
    }
    let text = unsafe { (shared.value_to_string)(value) };
    if text.is_null() {
        return;
    }
    let message = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
    unsafe { (shared.g_free)(text.cast()) };
    unsafe { (shared.result_unref)(result) };
    if let Ok(mut queue) = shared.queue.lock() {
        queue.push_back(message);
    }
}

unsafe extern "C" fn on_destroy(_widget: *mut c_void, data: *mut c_void) {
    if data.is_null() {
        return;
    }
    let shared = unsafe { &*(data as *const Shared) };
    shared.alive.store(false, Ordering::SeqCst);
}

/// Linux WebKitGTK 后端。
pub struct LinuxBackend {
    api: GtkApi,
    _libs: Libs,
    window: *mut GtkWidget,
    webview: *mut WebKitWebView,
    shared: Arc<Shared>,
    shared_raw: *mut c_void,
    handler: Box<dyn IpcHandler>,
}

impl LinuxBackend {
    fn cstring(text: &str, what: &str) -> Result<CString> {
        CString::new(text).map_err(|_| WebViewError::Message(format!("{what} 含有 NUL 字节")))
    }
}

impl WebViewBackend for LinuxBackend {
    const NAME: &'static str = "linux";

    fn open(options: OpenOptions, handler: Box<dyn IpcHandler>) -> Result<Self> {
        if options.content_dir.is_none() && options.url.is_none() {
            return Err(WebViewError::Message("打开 WebView 需要 content_dir 或 url".into()));
        }
        let libs = Libs::load()?;

        let mut argc: c_int = 0;
        let mut argv = [ptr::null_mut()];
        let mut argv_ptr = argv.as_mut_ptr();
        // SAFETY: 传入合法的 argc/argv 缓冲区；GTK 要求在同一线程后续驱动事件循环。
        if unsafe { (libs.api.gtk_init_check)(&mut argc, &mut argv_ptr) } == 0 {
            return Err(WebViewError::host(
                "gtk_init_failed",
                "gtk_init",
                "GTK 初始化失败（缺少图形显示环境或 DISPLAY / WAYLAND_DISPLAY 未设置）",
            ));
        }

        let shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            alive: AtomicBool::new(true),
            result_get_value: libs.api.webkit_javascript_result_get_js_value,
            value_to_string: libs.api.jsc_value_to_string,
            result_unref: libs.api.webkit_javascript_result_unref,
            g_free: libs.api.g_free,
        });
        let shared_raw = Arc::into_raw(shared.clone()).cast_mut().cast::<c_void>();

        let title = Self::cstring(&options.title, "窗口标题")?;
        // SAFETY: GTK 对象均在 `gtk_init_check` 成功后创建；信号回调指针在结构销毁前保持有效。
        let (window, webview) = unsafe {
            let window = (libs.api.gtk_window_new)(GTK_WINDOW_TOPLEVEL);
            if window.is_null() {
                return Err(WebViewError::Message("创建 GTK 窗口失败".into()));
            }
            (libs.api.gtk_window_set_title)(window, title.as_ptr());
            (libs.api.gtk_window_set_default_size)(window, options.width, options.height);
            (libs.api.gtk_window_set_position)(window, GTK_WIN_POS_CENTER);

            let webview = (libs.api.webkit_web_view_new)();
            if webview.is_null() {
                (libs.api.gtk_widget_destroy)(window);
                return Err(WebViewError::Message("创建 WebKitWebView 失败".into()));
            }
            let manager = (libs.api.webkit_web_view_get_user_content_manager)(webview);
            let handler_name = Self::cstring(MESSAGE_HANDLER_NAME, "消息处理器名")?;
            if (libs.api.webkit_user_content_manager_register_script_message_handler)(manager, handler_name.as_ptr()) == 0 {
                (libs.api.gtk_widget_destroy)(window);
                return Err(WebViewError::Message("WebKitGTK 注册消息处理器失败".into()));
            }

            let signal = c"script-message-received::wae";
            (libs.api.g_signal_connect_data)(manager.cast(), signal.as_ptr(), on_script_message as *mut c_void, shared_raw, None, 0);
            (libs.api.g_signal_connect_data)(window.cast(), c"destroy".as_ptr(), on_destroy as *mut c_void, shared_raw, None, 0);

            (libs.api.gtk_container_add)(window, webview.cast());
            (libs.api.gtk_widget_show_all)(window);
            (window, webview)
        };

        let target = if let Some(dir) = &options.content_dir {
            let dir = std::fs::canonicalize(dir).map_err(|error| WebViewError::Message(format!("content_dir 无法解析: {error}")))?;
            let index = dir.join("index.html");
            if !index.is_file() {
                return Err(WebViewError::Message(format!("content_dir 缺少 index.html: {}", index.display())));
            }
            file_url(&index)
        }
        else {
            options.url.clone().unwrap_or_default()
        };
        let target = Self::cstring(&target, "加载地址")?;
        // SAFETY: `webview` 已创建且未销毁。
        unsafe { (libs.api.webkit_web_view_load_uri)(webview, target.as_ptr()) };

        Ok(Self { api: libs.api, _libs: libs, window, webview, shared, shared_raw, handler })
    }

    fn pump(&mut self) -> bool {
        if !self.shared.alive.load(Ordering::SeqCst) {
            return false;
        }
        // SAFETY: 与 `gtk_init_check` 同线程驱动 GTK 事件循环。
        unsafe {
            while (self.api.gtk_events_pending)() != 0 {
                (self.api.gtk_main_iteration_do)(0);
            }
        }

        loop {
            let message = self.shared.queue.lock().ok().and_then(|mut queue| queue.pop_front());
            let Some(message) = message
            else {
                break;
            };
            match self.handler.on_ipc(&message) {
                IpcOutcome::ReplyToPage(json) => {
                    if self.post_message(&json).is_err() {
                        break;
                    }
                }
                IpcOutcome::CloseWindow => {
                    let _ = self.close();
                    return false;
                }
                IpcOutcome::Ignored => {}
            }
        }

        self.shared.alive.load(Ordering::SeqCst)
    }

    fn close(&mut self) -> Result<()> {
        if self.shared.alive.swap(false, Ordering::SeqCst) {
            // SAFETY: 窗口由本结构创建，且仅在仍存活时销毁。
            unsafe { (self.api.gtk_widget_destroy)(self.window) };
        }
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }

    fn focus(&mut self) -> Result<()> {
        if !self.shared.alive.load(Ordering::SeqCst) {
            return Err(WebViewError::host("window_not_alive", "focus", "窗口已关闭，无法唤醒"));
        }
        // SAFETY: 窗口由本结构创建且仍存活。
        unsafe { (self.api.gtk_window_present)(self.window) };
        Ok(())
    }

    fn post_message(&self, json: &str) -> Result<()> {
        if !self.shared.alive.load(Ordering::SeqCst) {
            return Err(WebViewError::Message("Linux WebView 窗口已关闭".into()));
        }
        let script = Self::cstring(&page_receive_script(json), "回投脚本")?;
        // SAFETY: `webview` 存活；长度传 -1 表示 NUL 结尾。
        unsafe {
            (self.api.webkit_web_view_evaluate_javascript)(
                self.webview,
                script.as_ptr(),
                -1,
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
                None,
                ptr::null_mut(),
            );
        }
        Ok(())
    }
}

impl Drop for LinuxBackend {
    fn drop(&mut self) {
        let _ = self.close();
        // SAFETY: `shared_raw` 来自 `Arc::into_raw` 的一次独立引用，只在此回收一次。
        drop(unsafe { Arc::from_raw(self.shared_raw.cast::<Shared>()) });
    }
}

/// 把本地路径转成 WebKitGTK 可加载的 `file://` URL（仅保留 RFC 3986 unreserved 与 `/`）。
fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut url = String::from("file://");
    for byte in text.as_bytes() {
        let b = *byte;
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/');
        if keep {
            url.push(char::from(b));
        }
        else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_url_encodes_reserved_bytes() {
        assert_eq!(file_url(Path::new("/tmp/wae app/index.html")), "file:///tmp/wae%20app/index.html");
        assert_eq!(file_url(Path::new("/tmp/wae/#1.html")), "file:///tmp/wae/%231.html");
    }

    #[test]
    fn missing_runtime_libraries_report_a_dependency_failure() {
        let error = match open_first(&["libwae-missing-runtime-probe.so"]) {
            Ok(_) => panic!("a missing library must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), Some("webkit_dependency_missing"));
        assert_eq!(error.stage(), Some("load_libraries"));
        match &error {
            WebViewError::Host(failure) => {
                assert!(failure.attempted.iter().any(|entry| entry.contains("libwae-missing-runtime-probe.so")));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn gtk_and_webkit_symbols_resolve() {
        if let Err(error) = Libs::load() {
            panic!("所需 GTK3 / WebKitGTK 4.1 运行库不可用: {error}");
        }
    }
}
