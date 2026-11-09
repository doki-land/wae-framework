//! 门面跨平台符号门禁：三个桌面 target 上同一组符号必须存在且签名一致。
//!
//! 本文件刻意不出现任何平台 `cfg`，代表下游应用（含 `napi` 层）应有的写法。

use wae::webview::{Capability, IpcHandler, IpcOutcome, OpenOptions, Platform, Result, WebViewError, WebViewHost, current_platform, open};

struct NoopHandler;

impl IpcHandler for NoopHandler {
    fn on_ipc(&mut self, _message: &str) -> IpcOutcome {
        IpcOutcome::Ignored
    }
}

/// 模拟应用侧的一行转发：不得因平台不同而分叉。
fn forward_open(options: OpenOptions) -> Result<WebViewHost> {
    open(options, Box::new(NoopHandler))
}

#[test]
fn facade_symbols_exist_with_stable_signatures() {
    let _: fn(OpenOptions, Box<dyn IpcHandler>) -> Result<WebViewHost> = WebViewHost::open;
    let _: fn(OpenOptions) -> Result<WebViewHost> = forward_open;
    let _: fn(&mut WebViewHost) -> bool = WebViewHost::pump;
    let _: fn(&mut WebViewHost) -> Result<()> = WebViewHost::close;
    let _: fn(&WebViewHost, &str) -> Result<()> = WebViewHost::post_message;
    let _: fn(&WebViewHost) -> bool = WebViewHost::is_alive;
    let _: fn(&WebViewHost) -> &'static str = WebViewHost::backend_name;
    let _: fn() -> Platform = current_platform;
}

/// shell 门面在三个桌面 target 上同一签名，下游不得平台分叉。
#[test]
fn shell_surface_matches_all_desktop_targets() {
    let _: fn(&std::path::Path) -> std::result::Result<(), wae::shell::ShellError> = wae::shell::open_path;
}

#[test]
fn unsupported_error_carries_capability_and_platform() {
    let error = WebViewError::unsupported(Capability::MessageChannel, current_platform());
    assert_eq!(error.capability(), Some(Capability::MessageChannel));
    assert_eq!(error.platform(), Some(current_platform()));
    match &error {
        WebViewError::Unsupported { capability, platform } => {
            assert_eq!(*capability, Capability::MessageChannel);
            assert_eq!(*platform, current_platform());
        }
        WebViewError::Message(message) => panic!("unexpected message error: {message}"),
        WebViewError::Host(failure) => panic!("unexpected host error: {failure:?}"),
    }
}

#[test]
fn platform_matches_compiled_target() {
    let expected = if cfg!(target_os = "windows") {
        Platform::Windows
    }
    else if cfg!(target_os = "macos") {
        Platform::Macos
    }
    else {
        Platform::Linux
    };
    assert_eq!(current_platform(), expected);
}
