//! 临时诊断：虚拟主机对无扩展名路径返回什么（是否 404、`IsSuccess` 真假）。

#[cfg(windows)]
fn main() {
    use std::{
        fs,
        thread::sleep,
        time::{Duration, Instant},
    };

    use wae_webview::{IpcHandler, IpcOutcome, OpenOptions, WebViewBackend};
    use wae_webview_win32::Win32Backend;

    struct Ignore;

    impl IpcHandler for Ignore {
        fn on_ipc(&mut self, message: &str) -> IpcOutcome {
            eprintln!("[probe] page={message}");
            IpcOutcome::Ignored
        }
    }

    let route = std::env::args().nth(1).unwrap_or_else(|| "/charts".to_string());
    let page_dir = std::env::temp_dir().join(format!("wae-route-probe-{}", std::process::id()));
    let _ = fs::remove_dir_all(&page_dir);
    fs::create_dir_all(&page_dir).expect("create page dir");
    fs::write(
        page_dir.join("index.html"),
        "<!doctype html><title>ROOT</title><p>root</p><script>window.chrome.webview.postMessage('loaded path=' + location.pathname);</script>",
    )
    .expect("write index");

    unsafe {
        std::env::set_var("WAE_WEBVIEW_DIAG", "1");
    }
    let url = if route.starts_with("http") { route.clone() } else { format!("https://app.local{route}") };
    let options = OpenOptions::from_url(&url, "route probe");
    let _ = &options;
    // 既映射目录又导航到指定路径：先把 url 换成内容目录再改回目标地址不可行，
    // 因此直接打开内容目录，再用 url 覆盖导航目标。
    let mut options = OpenOptions::from_content_dir(page_dir.to_string_lossy().to_string(), "route probe");
    options.url = Some(url.clone());
    let mut backend = Win32Backend::open(options, Box::new(Ignore)).expect("open probe window");
    let started = Instant::now();
    while backend.pump() {
        if started.elapsed() >= Duration::from_secs(8) {
            break;
        }
        sleep(Duration::from_millis(10));
    }
    let _ = backend.close();
    let _ = fs::remove_dir_all(&page_dir);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("route_probe is Windows only");
}
