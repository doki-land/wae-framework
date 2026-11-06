use std::{
    collections::VecDeque,
    ffi::{OsStr, c_void},
    os::windows::ffi::OsStrExt,
    path::Path,
    sync::{Mutex, mpsc},
};

use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW, CreateCoreWebView2EnvironmentWithOptions, GetAvailableCoreWebView2BrowserVersionString,
        ICoreWebView2, ICoreWebView2_3, ICoreWebView2Controller, ICoreWebView2Environment,
    },
    WebMessageReceivedEventHandler, wait_with_pump,
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
        UI::WindowsAndMessaging::{
            CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClientRect,
            GetWindowLongPtrW, IDC_ARROW, IsWindow, LoadCursorW, MSG, PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassW,
            SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
            WINDOW_EX_STYLE, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_QUIT, WM_SIZE, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
        },
    },
    core::{Interface, PCWSTR, PWSTR, w},
};

use wae_webview::{IpcHandler, IpcOutcome, OpenOptions, Result, WebViewBackend, WebViewError};

use crate::dpi;

type EventRegistrationToken = i64;

const WM_WAE_IPC: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;
const VIRTUAL_HOST: &str = "app.local";

struct DesktopState {
    options: OpenOptions,
    hwnd: HWND,
    controller: Option<ICoreWebView2Controller>,
    webview: Option<ICoreWebView2>,
    handler: Box<dyn IpcHandler>,
    pending_ipc: Mutex<VecDeque<String>>,
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
        let line = format!(
            "pump.exit reason={reason} pumps={} hwnd={:?} is_window={is_window} alive={} quit={}",
            self.pumps, self.state.hwnd, self.state.alive, self.quit
        );
        dpi::diag(line.clone());
        if self.pumps <= 2 {
            eprintln!("[wae-webview] unexpected_exit {line}");
        }
    }
}

impl WebViewBackend for Win32Backend {
    const NAME: &'static str = "win32";

    fn open(options: OpenOptions, handler: Box<dyn IpcHandler>) -> Result<Self> {
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

            let class_name = w!("WAE_OPEN_CLEANER_HOST");
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
