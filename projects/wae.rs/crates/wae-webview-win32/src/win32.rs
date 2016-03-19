use std::{
    collections::VecDeque,
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    path::Path,
    sync::{Mutex, mpsc},
};

use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW, CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2_3,
        ICoreWebView2Controller, ICoreWebView2Environment,
    },
    WebMessageReceivedEventHandler, wait_with_pump,
};
use windows::{
    Win32::{
        Foundation::{E_POINTER, E_UNEXPECTED, HWND, LPARAM, LRESULT, WPARAM},
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
            LibraryLoader::GetModuleHandleW,
        },
        UI::WindowsAndMessaging::{
            CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClientRect,
            GetWindowLongPtrW, IDC_ARROW, IsWindow, LoadCursorW, MSG, PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassW,
            SW_SHOW, SetForegroundWindow, SetWindowLongPtrW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WM_CLOSE, WM_CREATE, WM_DESTROY,
            WM_QUIT, WM_SIZE, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
        },
    },
    core::{Interface, PCWSTR, w},
};

use crate::{IpcHandler, IpcOutcome, OpenOptions, Result, WebViewError};

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
    }

    fn post_message(&self, json: &str) -> Result<()> {
        let Some(webview) = self.webview.as_ref()
        else {
            return Err(WebViewError::Message("webview not ready".into()));
        };
        let wide = encode_wide(json);
        unsafe {
            webview.PostWebMessageAsString(PCWSTR(wide.as_ptr())).map_err(|e| WebViewError::Message(e.to_string()))?;
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

fn create_environment() -> Result<ICoreWebView2Environment> {
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
        .map_err(|e| WebViewError::Message(e.to_string()))?;
    }
    wait_with_pump(rx).map_err(|e| WebViewError::Message(format!("{e:?}")))?.map_err(|e| WebViewError::Message(e.to_string()))
}

fn create_controller(hwnd: HWND, environment: &ICoreWebView2Environment) -> Result<ICoreWebView2Controller> {
    let (tx, rx) = mpsc::channel();
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |error_code, controller| {
        error_code?;
        tx.send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER))).map_err(|_| windows::core::Error::from(E_UNEXPECTED))
    }));
    unsafe {
        environment.CreateCoreWebView2Controller(hwnd, &handler).map_err(|e| WebViewError::Message(e.to_string()))?;
    }
    wait_with_pump(rx).map_err(|e| WebViewError::Message(format!("{e:?}")))?.map_err(|e| WebViewError::Message(e.to_string()))
}

fn attach_webview(state: *mut DesktopState, controller: ICoreWebView2Controller) -> Result<()> {
    let webview = unsafe { controller.CoreWebView2().map_err(|e| WebViewError::Message(e.to_string()))? };

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
        webview.add_WebMessageReceived(&message_handler, &mut token).map_err(|e| WebViewError::Message(e.to_string()))?;
    }

    let desktop = unsafe { &mut *state };
    let navigate_url = resolve_navigate_url(&desktop.options, &webview)?;

    desktop.controller = Some(controller);
    desktop.webview = Some(webview.clone());
    desktop.resize_webview();

    let wide_url = encode_wide(&navigate_url);
    unsafe {
        webview.Navigate(PCWSTR(wide_url.as_ptr())).map_err(|e| WebViewError::Message(format!("Navigate({navigate_url}) failed: {e}")))?;
    }

    Ok(())
}

fn resolve_navigate_url(options: &OpenOptions, webview: &ICoreWebView2) -> Result<String> {
    if let Some(dir) = options.content_dir.as_ref() {
        let folder = Path::new(dir).canonicalize().map_err(|e| WebViewError::Message(format!("content_dir {dir}: {e}")))?;
        if !folder.is_dir() {
            return Err(WebViewError::Message(format!("content_dir is not a directory: {}", folder.display())));
        }
        let folder_s = strip_extended_path(&folder);
        let host = encode_wide(VIRTUAL_HOST);
        let folder_w = encode_wide(&folder_s);
        let webview3: ICoreWebView2_3 = webview.cast().map_err(|e| WebViewError::Message(format!("ICoreWebView2_3 cast failed: {e}")))?;
        unsafe {
            webview3
                .SetVirtualHostNameToFolderMapping(
                    PCWSTR(host.as_ptr()),
                    PCWSTR(folder_w.as_ptr()),
                    COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW,
                )
                .map_err(|e| WebViewError::Message(format!("SetVirtualHostNameToFolderMapping failed: {e}")))?;
        }
        return Ok(format!("https://{VIRTUAL_HOST}/index.html"));
    }

    options.url.clone().ok_or_else(|| WebViewError::Message("OpenOptions needs content_dir or url".into()))
}

fn strip_extended_path(path: &Path) -> String {
    let mut s = path.display().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        s = rest.to_string();
    }
    s
}

pub struct HostInner {
    state: Box<DesktopState>,
    quit: bool,
}

impl HostInner {
    pub fn open(options: OpenOptions, handler: Box<dyn IpcHandler>) -> Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            let class_name = w!("WAE_OPEN_CLEANER_HOST");
            let instance = GetModuleHandleW(None).map_err(|e| WebViewError::Message(e.to_string()))?;

            let wc = WNDCLASSW {
                hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| WebViewError::Message(e.to_string()))?,
                hInstance: instance.into(),
                lpszClassName: class_name,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wnd_proc),
                ..Default::default()
            };
            // Ignore "class already exists" on repeated opens in one process.
            RegisterClassW(&wc);

            let title = encode_wide(&options.title);
            let width = options.width;
            let height = options.height;
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
                100,
                100,
                width,
                height,
                None,
                None,
                Some(instance.into()),
                Some(box_state.as_mut() as *mut DesktopState as *mut _),
            )
            .map_err(|e| WebViewError::Message(format!("CreateWindowExW failed: {e}")))?;

            box_state.hwnd = hwnd;
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);

            let environment = create_environment()?;
            let controller = create_controller(hwnd, &environment)?;
            attach_webview(box_state.as_mut(), controller)?;

            Ok(Self { state: box_state, quit: false })
        }
    }

    pub fn pump(&mut self) -> bool {
        if self.quit || !self.state.alive {
            self.quit = true;
            return false;
        }

        if !unsafe { IsWindow(Some(self.state.hwnd)).as_bool() } {
            self.quit = true;
            self.state.alive = false;
            return false;
        }

        unsafe {
            let mut msg = MSG::default();
            // 先抽线程级 WM_QUIT，再处理窗口消息。
            if PeekMessageW(&mut msg, None, WM_QUIT, WM_QUIT, PM_REMOVE).as_bool() {
                self.quit = true;
                self.state.alive = false;
                return false;
            }
            while PeekMessageW(&mut msg, Some(self.state.hwnd), 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                if !self.state.alive {
                    self.quit = true;
                    return false;
                }
            }
            // WebView2 / COM also posts thread messages with hwnd=NULL (not WM_QUIT).
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    self.quit = true;
                    self.state.alive = false;
                    return false;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                if !self.state.alive {
                    self.quit = true;
                    return false;
                }
            }
        }

        self.state.alive && !self.quit
    }

    pub fn close(&mut self) -> Result<()> {
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

    pub fn post_message(&self, json: &str) -> Result<()> {
        self.state.post_message(json)
    }

    pub fn is_alive(&self) -> bool {
        self.state.alive && !self.quit
    }
}
