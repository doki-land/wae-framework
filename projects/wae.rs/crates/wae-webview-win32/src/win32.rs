use std::{
    collections::VecDeque,
    ffi::{OsStr, c_void},
    os::windows::ffi::OsStrExt,
    path::Path,
    sync::{Mutex, mpsc},
};

use webview2_com::{
    AcceleratorKeyPressedEventHandler, AddScriptToExecuteOnDocumentCreatedCompletedHandler, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW, COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
        COREWEBVIEW2_WEB_ERROR_STATUS, COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT,
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT, COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED,
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID, COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED,
        COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS, COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED,
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET, COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED,
        COREWEBVIEW2_WEB_ERROR_STATUS_ERROR_HTTP_INVALID_SERVER_RESPONSE, COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED,
        COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED, COREWEBVIEW2_WEB_ERROR_STATUS_REDIRECT_FAILED,
        COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE, COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT,
        COREWEBVIEW2_WEB_ERROR_STATUS_UNEXPECTED_ERROR, COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_AUTHENTICATION_CREDENTIALS_REQUIRED,
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED, CreateCoreWebView2EnvironmentWithOptions,
        GetAvailableCoreWebView2BrowserVersionString, ICoreWebView2, ICoreWebView2_3, ICoreWebView2Controller, ICoreWebView2Environment,
    },
    NavigationCompletedEventHandler, NavigationStartingEventHandler, WebMessageReceivedEventHandler, wait_with_pump,
};
use windows::{
    Win32::{
        Foundation::{
            E_POINTER, E_UNEXPECTED, ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, RECT, RPC_E_CHANGED_MODE, WPARAM,
        },
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree},
            LibraryLoader::GetModuleHandleW,
        },
        UI::{
            Input::KeyboardAndMouse::VK_F12,
            WindowsAndMessaging::{
                CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClientRect,
                GetWindowLongPtrW, IDC_ARROW, IsWindow, LoadCursorW, MSG, PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage,
                RegisterClassW, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow,
                TranslateMessage, WINDOW_EX_STYLE, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_KEYDOWN, WM_QUIT, WM_SIZE, WNDCLASSW,
                WS_OVERLAPPEDWINDOW, WS_VISIBLE,
            },
        },
    },
    core::{Interface, PCWSTR, PWSTR, w},
};

use wae_webview::{IpcHandler, IpcOutcome, OpenOptions, Result, WebViewBackend, WebViewError};

use crate::{console, dpi};

type EventRegistrationToken = i64;

const WM_WAE_IPC: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;
const VIRTUAL_HOST: &str = "app.local";
/// 回退时携带原始路由的查询参数名，由前置脚本在页面脚本运行前消费。
const SPA_ROUTE_PARAM: &str = "__wae_route";

struct DesktopState {
    options: OpenOptions,
    hwnd: HWND,
    controller: Option<ICoreWebView2Controller>,
    webview: Option<ICoreWebView2>,
    handler: Box<dyn IpcHandler>,
    pending_ipc: Mutex<VecDeque<String>>,
    /// 最近一次导航失败的结构化错误，用于退出时复述并解释白屏。
    navigation_error: Mutex<Option<WebViewError>>,
    alive: bool,
}

impl DesktopState {
    fn dispatch_ipc(&mut self) {
        let message = {
            let mut guard = self.pending_ipc.lock().expect("ipc lock");
            guard.pop_front()
        };
        let Some(message) = message
        else {
            return;
        };

        match self.handler.on_ipc(&message) {
            IpcOutcome::ReplyToPage(json) => {
                let _ = self.post_message(&json);
            }
            IpcOutcome::CloseWindow => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
            IpcOutcome::Ignored => {}
        }
    }

    fn resize_webview(&self) {
        let Some(controller) = self.controller.as_ref()
        else {
            return;
        };
        unsafe {
            let mut rect = windows::Win32::Foundation::RECT::default();
            if GetClientRect(self.hwnd, &mut rect).is_err() {
                return;
            }
            let _ = controller.SetBounds(rect);
        }
        self.log_geometry("resize");
    }

    /// 打开诊断开关时打印窗口 DPI、客户区物理/逻辑尺寸与 controller 边界。
    ///
    /// WebView2 的 `SetBounds` 单位是相对父窗口客户区的**物理像素**；
    /// 客户区来自 `GetClientRect`（Per-Monitor V2 进程返回物理像素）。
    fn log_geometry(&self, event: &str) {
        if !dpi::diag_enabled() {
            return;
        }
        let mut client = RECT::default();
        let client_ok = unsafe { GetClientRect(self.hwnd, &mut client) }.is_ok();
        let window_dpi = dpi::window_dpi(self.hwnd);
        let mut bounds = RECT::default();
        let bounds_ok = self.controller.as_ref().map(|controller| unsafe { controller.Bounds(&mut bounds).is_ok() }).unwrap_or(false);
        let bounds_text = if bounds_ok {
            format!("{}x{}@{},{}", bounds.right - bounds.left, bounds.bottom - bounds.top, bounds.left, bounds.top)
        }
        else {
            "none".to_string()
        };
        let client_text =
            if client_ok { format!("{}x{}", client.right - client.left, client.bottom - client.top) } else { "unknown".to_string() };
        dpi::diag(format!(
            "dpi.geometry event={event} dpi={window_dpi} scale={:.2} client_physical={client_text} client_logical={:.1}x{:.1} controller_bounds={bounds_text}",
            f64::from(window_dpi) / 96.0,
            dpi::physical_to_dip(client.right - client.left, window_dpi),
            dpi::physical_to_dip(client.bottom - client.top, window_dpi),
        ));
    }

    /// 记录 WebView2 导航结果；失败立即输出结构化错误，并在退出时复述。
    ///
    /// 没有这条记录时，映射或路径问题只表现为 Edge 错误页，进程仍认为窗口
    /// 正常打开，CLI 只打印 `closed (win32)`。
    fn report_navigation(&self, success: bool, status: COREWEBVIEW2_WEB_ERROR_STATUS, url: &str) {
        if success {
            dpi::diag(format!("navigation.completed ok url={url}"));
            return;
        }
        // 初始 `about:blank` 被真实导航取代、以及 SPA 回退主动取消的导航都会以
        // `operation_canceled` 收尾，这不是页面加载失败，不制造噪音也不进退出摘要。
        if status == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED || url == "about:blank" {
            dpi::diag(format!("navigation.canceled url={url} web_error_status={}", web_error_status_name(status)));
            return;
        }
        let error = WebViewError::host(
            "webview2_navigation_failed",
            "navigate",
            format!("页面未加载: url={url} web_error_status={}({})", web_error_status_name(status), status.0),
        )
        .with_attempted(vec![url.to_string()]);
        dpi::emit(format!("navigation.failed {error}"));
        // 同一次加载可能产生多个 NavigationCompleted（首次尝试与重试），
        // 退出摘要取最后一个，它才是最终失败原因。
        *self.navigation_error.lock().expect("navigation lock") = Some(error);
    }

    fn post_message(&self, json: &str) -> Result<()> {
        let Some(webview) = self.webview.as_ref()
        else {
            return Err(WebViewError::host("webview_not_ready", "post_message", "webview 尚未就绪"));
        };
        let wide = encode_wide(json);
        unsafe {
            webview.PostWebMessageAsString(PCWSTR(wide.as_ptr())).map_err(|e| host_failure("post_message_failed", "post_message", &e))?;
        }
        Ok(())
    }

    /// 打开 WebView2 DevTools（独立窗口）。
    ///
    /// 可见控制台不再是调试入口：`F12` 与 `WAE_WEBVIEW_DIAG=1` 都经这里打开
    /// DevTools，页面控制台、网络与 DOM 检查照旧可用。
    fn open_dev_tools(&self, reason: &str) {
        let Some(webview) = self.webview.as_ref()
        else {
            dpi::emit(format!("devtools.open_failed reason={reason} error=webview_not_ready"));
            return;
        };
        match unsafe { webview.OpenDevToolsWindow() } {
            Ok(()) => dpi::emit(format!("devtools.open reason={reason}")),
            Err(error) => dpi::emit(format!("devtools.open_failed reason={reason} hresult={:#x} message={}", error.code().0, error.message())),
        }
    }
}

fn encode_wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_CREATE {
            let create = lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            let state = (*create).lpCreateParams as *mut DesktopState;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
            (*state).hwnd = hwnd;
            return LRESULT(0);
        }

        let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DesktopState;
        if state_ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let state = &mut *state_ptr;

        match msg {
            WM_SIZE => {
                state.resize_webview();
                LRESULT(0)
            }
            WM_DPICHANGED => {
                // 跨显示器（或系统缩放变化）后使用系统建议矩形，客户区随之变为新
                // DPI 下的物理像素，WM_SIZE 会重新计算 WebView2 边界。
                let suggested = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                state.log_geometry("wm_dpichanged");
                LRESULT(0)
            }
            WM_WAE_IPC => {
                state.dispatch_ipc();
                LRESULT(0)
            }
            // 启动后键盘焦点停在顶层窗口（WebView2 子窗口未取得焦点），
            // `AcceleratorKeyPressed` 收不到按键。这条分支覆盖该实测路径，
            // 与 accelerator 分支互补，任一焦点位置按 F12 都能开 DevTools。
            WM_KEYDOWN if wparam.0 == usize::from(VK_F12.0) => {
                state.open_dev_tools("f12");
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                state.alive = false;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn host_failure(code: &'static str, stage: &'static str, error: &windows::core::Error) -> WebViewError {
    WebViewError::host(code, stage, error.message()).with_hresult(error.code().0)
}

/// `COREWEBVIEW2_WEB_ERROR_STATUS` 的可读名字，避免日志里只剩一个裸数字。
fn web_error_status_name(status: COREWEBVIEW2_WEB_ERROR_STATUS) -> &'static str {
    match status {
        COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN => "unknown",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT => "certificate_common_name_is_incorrect",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED => "certificate_expired",
        COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS => "client_certificate_contains_errors",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED => "certificate_revoked",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID => "certificate_is_invalid",
        COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE => "server_unreachable",
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => "timeout",
        COREWEBVIEW2_WEB_ERROR_STATUS_ERROR_HTTP_INVALID_SERVER_RESPONSE => "error_http_invalid_server_response",
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED => "connection_aborted",
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET => "connection_reset",
        COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED => "disconnected",
        COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT => "cannot_connect",
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => "host_name_not_resolved",
        COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED => "operation_canceled",
        COREWEBVIEW2_WEB_ERROR_STATUS_REDIRECT_FAILED => "redirect_failed",
        COREWEBVIEW2_WEB_ERROR_STATUS_UNEXPECTED_ERROR => "unexpected_error",
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_AUTHENTICATION_CREDENTIALS_REQUIRED => "valid_authentication_credentials_required",
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED => "valid_proxy_authentication_required",
        _ => "unrecognized_status",
    }
}

/// WebView2 Evergreen Runtime 的常见探测位置，用于「运行时缺失」的排障提示。
fn runtime_probe_paths() -> Vec<String> {
    let mut paths = vec![
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}\pv".to_string(),
        r"HKCU\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}\pv".to_string(),
    ];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        paths.push(format!("{}\\Microsoft\\EdgeWebView\\Application", Path::new(&local).display()));
    }
    paths
}

/// 创建环境前先确认 WebView2 Runtime 存在；缺失直接给结构化错误，不让进程静默退出。
fn webview2_runtime_version() -> Result<String> {
    let mut version = PWSTR::null();
    let probe = unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version) };
    if probe.is_ok() {
        let text = unsafe { version.to_string() }.unwrap_or_default();
        if !version.is_null() {
            unsafe { CoTaskMemFree(Some(version.as_ptr() as *const c_void)) };
        }
        dpi::diag(format!("webview2.runtime version={text}"));
        return Ok(text);
    }

    let error = probe.expect_err("probe failed");
    Err(WebViewError::host(
        "webview2_runtime_missing",
        "resolve_runtime",
        "未检测到 WebView2 Runtime，页面无法渲染；请安装 Microsoft Edge WebView2 Runtime（Evergreen）后重试",
    )
    .with_hresult(error.code().0)
    .with_attempted(runtime_probe_paths()))
}

fn create_environment() -> Result<ICoreWebView2Environment> {
    let _ = webview2_runtime_version()?;
    let (tx, rx) = mpsc::channel();
    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            PCWSTR::null(),
            None,
            &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |error_code, environment| {
                error_code?;
                tx.send(environment.ok_or_else(|| windows::core::Error::from(E_POINTER))).map_err(|_| windows::core::Error::from(E_UNEXPECTED))
            })),
        )
        .map_err(|e| host_failure("webview2_environment_failed", "create_environment", &e))?;
    }
    wait_with_pump(rx)
        .map_err(|e| WebViewError::host("webview2_environment_failed", "create_environment", format!("等待环境创建消息失败: {e:?}")))?
        .map_err(|e| host_failure("webview2_environment_failed", "create_environment", &e))
}

fn create_controller(hwnd: HWND, environment: &ICoreWebView2Environment) -> Result<ICoreWebView2Controller> {
    let (tx, rx) = mpsc::channel();
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |error_code, controller| {
        error_code?;
        tx.send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER))).map_err(|_| windows::core::Error::from(E_UNEXPECTED))
    }));
    unsafe {
        environment
            .CreateCoreWebView2Controller(hwnd, &handler)
            .map_err(|e| host_failure("webview2_controller_failed", "create_controller", &e))?;
    }
    wait_with_pump(rx)
        .map_err(|e| WebViewError::host("webview2_controller_failed", "create_controller", format!("等待 controller 创建消息失败: {e:?}")))?
        .map_err(|e| host_failure("webview2_controller_failed", "create_controller", &e))
}

fn attach_webview(state: *mut DesktopState, controller: ICoreWebView2Controller) -> Result<()> {
    let webview = unsafe { controller.CoreWebView2().map_err(|e| host_failure("webview2_controller_failed", "read_core_webview", &e))? };

    let state_addr = state as usize;
    let mut token = EventRegistrationToken::default();
    let message_handler = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args
        else {
            return Ok(());
        };
        let mut message = windows::core::PWSTR::null();
        unsafe {
            args.TryGetWebMessageAsString(&mut message)?;
        }
        let text = unsafe { message.to_string().unwrap_or_default() };
        let desktop = unsafe { &*(state_addr as *const DesktopState) };
        {
            let mut guard = desktop.pending_ipc.lock().expect("ipc lock");
            guard.push_back(text);
        }
        unsafe {
            let _ = PostMessageW(Some(desktop.hwnd), WM_WAE_IPC, WPARAM(0), LPARAM(0));
        }
        Ok(())
    }));

    unsafe {
        webview
            .add_WebMessageReceived(&message_handler, &mut token)
            .map_err(|e| host_failure("webview2_message_channel_failed", "attach_message_channel", &e))?;
    }

    // 导航失败是「窗口在、页面白」的唯一可观测信号，必须订阅并结构化记录。
    let mut navigation_token = EventRegistrationToken::default();
    let navigation_handler = NavigationCompletedEventHandler::create(Box::new(move |sender, args| {
        let (Some(sender), Some(args)) = (sender, args)
        else {
            return Ok(());
        };
        let mut success = windows::core::BOOL::default();
        unsafe {
            args.IsSuccess(&mut success)?;
        }
        let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
        unsafe {
            args.WebErrorStatus(&mut status)?;
        }
        let mut source = PWSTR::null();
        let url = unsafe { sender.Source(&mut source) }.ok().and_then(|()| unsafe { source.to_string() }.ok()).unwrap_or_default();
        let desktop = unsafe { &*(state_addr as *const DesktopState) };
        desktop.report_navigation(success.as_bool(), status, &url);
        Ok(())
    }));

    unsafe {
        webview
            .add_NavigationCompleted(&navigation_handler, &mut navigation_token)
            .map_err(|e| host_failure("webview2_navigation_events_failed", "attach_navigation_events", &e))?;
    }

    // F12 是宿主约定：WebView2 先把按键交给宿主（`AcceleratorKeyPressed`），
    // 页面有焦点时也只有这里能拦到。DevTools 由此打开，不再依赖可见控制台。
    let mut accelerator_token = EventRegistrationToken::default();
    let accelerator_handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args
        else {
            return Ok(());
        };
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
        unsafe {
            args.KeyEventKind(&mut kind)?;
        }
        if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN {
            return Ok(());
        }
        let mut key = 0u32;
        unsafe {
            args.VirtualKey(&mut key)?;
        }
        if key != u32::from(VK_F12.0) {
            return Ok(());
        }
        let desktop = unsafe { &*(state_addr as *const DesktopState) };
        desktop.open_dev_tools("f12");
        unsafe {
            args.SetHandled(true)?;
        }
        Ok(())
    }));

    unsafe {
        controller
            .add_AcceleratorKeyPressed(&accelerator_handler, &mut accelerator_token)
            .map_err(|e| host_failure("webview2_accelerator_failed", "attach_accelerator", &e))?;
    }

    // 虚拟主机把不存在的无扩展名路径当作连接失败，SPA 直达路由因此白屏。
    // 回退挂在导航事件上，覆盖首屏、重载与站内链接跳转。
    attach_spa_fallback(state, &webview)?;

    let desktop = unsafe { &mut *state };
    let navigate_url = resolve_navigate_url(&desktop.options, &webview)?;

    desktop.controller = Some(controller);
    desktop.webview = Some(webview.clone());
    desktop.resize_webview();

    let wide_url = encode_wide(&navigate_url);
    unsafe {
        webview
            .Navigate(PCWSTR(wide_url.as_ptr()))
            .map_err(|e| host_failure("webview2_navigate_failed", "navigate", &e).with_attempted(vec![navigate_url.clone()]))?;
    }

    Ok(())
}

/// SPA 直达路由回退。
///
/// 虚拟主机把不存在的无扩展名路径当成连接失败（实测 `connection_aborted`），
/// 直达 `/charts` 这类路由会白屏。映射主机上的请求不会触发
/// `WebResourceRequested`（实测只有非映射主机触发），所以回退只能做在导航层：
/// 命中边界时取消这次导航，改为加载 `index.html`，并让文档创建后、页面脚本
/// 运行前把地址栏恢复成原始路径，前端仍按 `location.pathname` 路由。
///
/// 边界，三条同时满足才回退：
/// 1. 只处理 `app.local` 下的文档导航（文档导航只有 `GET`，子资源不经过这里）；
/// 2. 路径末段没有扩展名（`/assets/app.js` 之类原样交给虚拟主机）；
/// 3. 内容目录里确实没有该文件（真实存在的无扩展名文件照常加载）。
///
/// 挂在 `NavigationStarting` 上，因此首屏、页面重载与站内链接跳转都被覆盖；
/// 回退目标带 `.html` 扩展名，不会被再次命中，不存在回退环。
fn attach_spa_fallback(state: *mut DesktopState, webview: &ICoreWebView2) -> Result<()> {
    let desktop = unsafe { &*state };
    let Some(content_dir) = desktop.options.content_dir.clone()
    else {
        return Ok(());
    };
    let Ok(folder) = Path::new(&content_dir).canonicalize()
    else {
        return Ok(());
    };
    if !folder.join("index.html").is_file() {
        return Ok(());
    }

    install_spa_bootstrap(webview, &folder)?;

    let mut token = EventRegistrationToken::default();
    let folder_label = folder.display().to_string();
    let handler = NavigationStartingEventHandler::create(Box::new(move |sender, args| {
        let (Some(sender), Some(args)) = (sender, args)
        else {
            return Ok(());
        };
        let mut uri = PWSTR::null();
        unsafe {
            args.Uri(&mut uri)?;
        }
        let uri = unsafe { uri.to_string() }.unwrap_or_default();
        let Some(target) = spa_fallback_target(&folder, &uri)
        else {
            return Ok(());
        };
        unsafe {
            args.SetCancel(true)?;
            let wide = encode_wide(&target);
            sender.Navigate(PCWSTR(wide.as_ptr()))?;
        }
        dpi::diag(format!("spa.fallback uri={uri} target={target}"));
        Ok(())
    }));

    unsafe {
        webview.add_NavigationStarting(&handler, &mut token).map_err(|e| host_failure("webview2_fallback_failed", "attach_fallback", &e))?;
    }
    dpi::diag(format!("spa.fallback enabled folder={folder_label}"));
    Ok(())
}

/// 在文档脚本运行前把 `index.html?__wae_route=...` 的地址栏恢复成原始路由。
fn install_spa_bootstrap(webview: &ICoreWebView2, folder: &Path) -> Result<()> {
    let script = encode_wide(&spa_bootstrap_script());
    let handler = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(|_, _| Ok(())));
    unsafe {
        webview
            .AddScriptToExecuteOnDocumentCreated(PCWSTR(script.as_ptr()), &handler)
            .map_err(|e| host_failure("webview2_fallback_failed", "attach_bootstrap", &e))?;
    }
    dpi::diag(format!("spa.bootstrap attached folder={}", folder.display()));
    Ok(())
}

/// 前置脚本正文：把回退标记换回原始路由，其余查询参数与片段原样保留。
fn spa_bootstrap_script() -> String {
    format!(
        r#"(function () {{
    var marker = '{SPA_ROUTE_PARAM}=';
    var search = location.search || '';
    if (search.indexOf(marker) === -1) return;
    var parts = search.slice(1).split('&');
    var route = null;
    var rest = [];
    for (var i = 0; i < parts.length; i++) {{
        var part = parts[i];
        if (!part) continue;
        if (part.indexOf(marker) === 0) {{
            try {{ route = decodeURIComponent(part.slice(marker.length)); }} catch (error) {{ return; }}
        }} else {{
            rest.push(part);
        }}
    }}
    if (!route || route.charAt(0) !== '/') return;
    history.replaceState(history.state, '', route + (rest.length ? '?' + rest.join('&') : '') + (location.hash || ''));
}})();"#
    )
}

/// 命中回退边界时返回 `index.html` 加路由标记的地址。
fn spa_fallback_target(folder: &Path, uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("https://app.local")?;
    let path = rest.split(['?', '#']).next().unwrap_or(rest);
    if path.is_empty() || path.contains("..") || last_segment_has_extension(path) {
        return None;
    }
    if folder.join(path.trim_start_matches('/')).is_file() {
        return None;
    }
    Some(format!("https://{VIRTUAL_HOST}/index.html?{SPA_ROUTE_PARAM}={}", encode_route(path)))
}

/// 路径末段是否带扩展名；`/charts`、`/` 不带，`/main.js` 带。
fn last_segment_has_extension(path: &str) -> bool {
    path.rsplit('/').next().unwrap_or_default().contains('.')
}

/// 把路由路径编码进查询参数，只保留 RFC 3986 非保留字符与 `/`。
fn encode_route(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(byte as char);
            continue;
        }
        encoded.push('%');
        encoded.push_str(&format!("{byte:02X}"));
    }
    encoded
}

fn resolve_navigate_url(options: &OpenOptions, webview: &ICoreWebView2) -> Result<String> {
    if let Some(dir) = options.content_dir.as_ref() {
        let folder = Path::new(dir).canonicalize().map_err(|e| {
            WebViewError::host("content_dir_invalid", "resolve_content", format!("content_dir {dir}: {e}")).with_attempted(vec![dir.clone()])
        })?;
        if !folder.is_dir() {
            return Err(WebViewError::host(
                "content_dir_invalid",
                "resolve_content",
                format!("content_dir is not a directory: {}", folder.display()),
            )
            .with_attempted(vec![folder.display().to_string()]));
        }
        let folder_s = strip_extended_path(&folder);
        let host = encode_wide(VIRTUAL_HOST);
        let folder_w = encode_wide(&folder_s);
        let webview3: ICoreWebView2_3 = webview.cast().map_err(|e| host_failure("webview2_interface_missing", "map_content_dir", &e))?;
        unsafe {
            webview3
                .SetVirtualHostNameToFolderMapping(
                    PCWSTR(host.as_ptr()),
                    PCWSTR(folder_w.as_ptr()),
                    COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW,
                )
                .map_err(|e| host_failure("webview2_virtual_host_failed", "map_content_dir", &e).with_attempted(vec![folder_s.clone()]))?;
        }
        if let Some(url) = options.url.clone() {
            return Ok(url);
        }
        return Ok(format!("https://{VIRTUAL_HOST}/index.html"));
    }

    options.url.clone().ok_or_else(|| WebViewError::host("content_source_missing", "resolve_content", "OpenOptions 需要 content_dir 或 url"))
}

fn strip_extended_path(path: &Path) -> String {
    let mut s = path.display().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        s = rest.to_string();
    }
    s
}

/// Win32 + WebView2 后端。
pub struct Win32Backend {
    state: Box<DesktopState>,
    quit: bool,
    pumps: u64,
}

impl Win32Backend {
    /// 记录退出原因；首两次 pump 就退出属于异常，必须留下结构化痕迹。
    fn report_exit(&self, reason: &'static str) {
        let is_window = unsafe { IsWindow(Some(self.state.hwnd)).as_bool() };
        let navigation = self.state.navigation_error.lock().expect("navigation lock").as_ref().map(ToString::to_string);
        let line = format!(
            "pump.exit reason={reason} pumps={} hwnd={:?} is_window={is_window} alive={} quit={} navigation_error=\"{}\"",
            self.pumps,
            self.state.hwnd,
            self.state.alive,
            self.quit,
            navigation.as_deref().unwrap_or("none"),
        );
        dpi::diag(line.clone());
        if self.pumps <= 2 {
            dpi::emit(format!("unexpected_exit {line}"));
        }
    }
}

impl WebViewBackend for Win32Backend {
    const NAME: &'static str = "win32";

    fn open(options: OpenOptions, handler: Box<dyn IpcHandler>) -> Result<Self> {
        // 先装文件日志再收控制台：隐藏之后 stderr 不再有人看得见。
        dpi::install_log_sink();
        let console = console::hide_orphan_console();
        dpi::emit(format!("console.window present={} attached_processes={} hidden={}", console.window, console.processes, console.hidden));

        // 必须在任何窗口创建之前声明 Per-Monitor V2；晚于窗口创建则无效，
        // 进程仍会按位图整体拉伸，页面与文字发虚。
        let awareness = dpi::ensure_process_dpi_awareness();

        unsafe {
            // S_FALSE（已初始化）与 RPC_E_CHANGED_MODE（已按其它模型初始化）都不是致命错误：
            // WebView2 需要 STA，若线程已按 MTA 初始化，仍可继续并在 controller 阶段如实报错。
            let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            if com.is_err() && com != RPC_E_CHANGED_MODE {
                return Err(host_failure("com_initialize_failed", "initialize_com", &windows::core::Error::from_hresult(com)));
            }

            let class_name = w!("WAE_WEBVIEW_HOST");
            let instance = GetModuleHandleW(None).map_err(|e| host_failure("window_class_failed", "load_module", &e))?;

            let wc = WNDCLASSW {
                hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| host_failure("window_class_failed", "load_cursor", &e))?,
                hInstance: instance.into(),
                lpszClassName: class_name,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wnd_proc),
                ..Default::default()
            };
            // 同一进程内重复打开时类已存在，只有其它失败才算错误。
            if RegisterClassW(&wc) == 0 {
                let last = GetLastError();
                if last != ERROR_CLASS_ALREADY_EXISTS {
                    return Err(WebViewError::host("window_class_failed", "register_class", "RegisterClassW 失败").with_hresult(last.0 as i32));
                }
            }

            let title = encode_wide(&options.title);
            // 作者面传逻辑像素（DIP），窗口与 WebView2 边界全部按物理像素处理。
            let initial_dpi = dpi::system_dpi();
            let (width, height) =
                dpi::client_to_window_size(options.width, options.height, WS_OVERLAPPEDWINDOW, WINDOW_EX_STYLE(0), initial_dpi);
            let (width, height) = match dpi::primary_work_area() {
                Some(area) => (width.min(area.right - area.left), height.min(area.bottom - area.top)),
                None => (width, height),
            };
            let origin = dpi::dip_to_physical(100, initial_dpi);
            let mut box_state = Box::new(DesktopState {
                options,
                hwnd: HWND::default(),
                controller: None,
                webview: None,
                handler,
                pending_ipc: Mutex::new(VecDeque::new()),
                navigation_error: Mutex::new(None),
                alive: true,
            });

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class_name,
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                origin,
                origin,
                width,
                height,
                None,
                None,
                Some(instance.into()),
                Some(box_state.as_mut() as *mut DesktopState as *mut _),
            )
            .map_err(|e| host_failure("window_create_failed", "create_window", &e).with_attempted(vec![format!("class={}", VIRTUAL_HOST)]))?;

            box_state.hwnd = hwnd;
            dpi::diag(format!(
                "dpi.window created hwnd={hwnd:?} awareness={} window_dpi={} requested_physical={width}x{height}",
                awareness.name(),
                dpi::window_dpi(hwnd),
            ));
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);

            let environment = create_environment()?;
            let controller = create_controller(hwnd, &environment)?;
            attach_webview(box_state.as_mut(), controller)?;

            if dpi::diag_enabled() {
                box_state.open_dev_tools("diag");
            }

            Ok(Self { state: box_state, quit: false, pumps: 0 })
        }
    }

    fn pump(&mut self) -> bool {
        self.pumps += 1;

        if self.quit || !self.state.alive {
            self.quit = true;
            self.report_exit("state_not_alive");
            return false;
        }

        if !unsafe { IsWindow(Some(self.state.hwnd)).as_bool() } {
            self.quit = true;
            self.state.alive = false;
            self.report_exit("window_not_found");
            return false;
        }

        unsafe {
            let mut msg = MSG::default();
            // 先抽线程级 WM_QUIT，再处理窗口消息。
            if PeekMessageW(&mut msg, None, WM_QUIT, WM_QUIT, PM_REMOVE).as_bool() {
                self.quit = true;
                self.state.alive = false;
                self.report_exit("thread_wm_quit");
                return false;
            }
            while PeekMessageW(&mut msg, Some(self.state.hwnd), 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                if !self.state.alive {
                    self.quit = true;
                    self.report_exit("window_destroyed");
                    return false;
                }
            }
            // WebView2 / COM also posts thread messages with hwnd=NULL (not WM_QUIT).
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    self.quit = true;
                    self.state.alive = false;
                    self.report_exit("dispatched_wm_quit");
                    return false;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                if !self.state.alive {
                    self.quit = true;
                    self.report_exit("window_destroyed");
                    return false;
                }
            }
        }

        self.state.alive && !self.quit
    }

    fn close(&mut self) -> Result<()> {
        if !self.state.alive {
            return Ok(());
        }
        unsafe {
            let _ = DestroyWindow(self.state.hwnd);
        }
        // Pump once so WM_DESTROY runs.
        let _ = self.pump();
        Ok(())
    }

    fn post_message(&self, json: &str) -> Result<()> {
        self.state.post_message(json)
    }

    fn is_alive(&self) -> bool {
        self.state.alive && !self.quit
    }
}
