//! DPI 证据探针：用真实 Win32/WebView2 后端打开一张可测量的页面。
//!
//! 页面在画布上绘制**恰好 1 个设备像素**宽的黑色竖线，并通过 WebView2 消息通道
//! 回传 `window.devicePixelRatio` 等观测值；宿主侧在 `WAE_WEBVIEW_DIAG=1` 时
//! 打印窗口 DPI、客户区物理/逻辑尺寸与 controller 边界。
//!
//! ```text
//! cargo run --example dpi_probe -- --seconds 12
//! cargo run --example dpi_probe -- --awareness unaware --seconds 12
//! ```
//!
//! `--awareness unaware` 先固定进程为 DPI-unaware，用于复现修复前的位图拉伸。

#[cfg(windows)]
fn main() {
    use std::{
        fs,
        time::{Duration, Instant},
    };

    use wae_webview::{IpcHandler, IpcOutcome, IpcSink, OpenOptions, WebViewBackend};
    use wae_webview_win32::Win32Backend;
    use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_UNAWARE, SetProcessDpiAwarenessContext};

    struct PrintHandler;

    impl IpcHandler for PrintHandler {
        fn on_ipc(&mut self, message: &str, _sink: &IpcSink) -> IpcOutcome {
            eprintln!("[wae-dpi-probe] page={message}");
            IpcOutcome::Ignored
        }
    }

    let mut awareness = "pmv2".to_string();
    let mut seconds = 12u64;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--awareness" => awareness = args.next().unwrap_or(awareness),
            "--seconds" => seconds = args.next().and_then(|value| value.parse().ok()).unwrap_or(seconds),
            other => panic!("unknown argument: {other}"),
        }
    }

    unsafe {
        std::env::set_var("WAE_WEBVIEW_DIAG", "1");
    }
    if awareness == "unaware" {
        // 复现修复前状态：先固定为 unaware，后端随后的 Per-Monitor V2 请求会被拒绝。
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_UNAWARE);
        }
    }

    let page_dir = std::env::temp_dir().join(format!("wae-dpi-probe-{}", std::process::id()));
    let _ = fs::remove_dir_all(&page_dir);
    fs::create_dir_all(&page_dir).expect("create probe page dir");
    fs::write(page_dir.join("index.html"), PROBE_HTML).expect("write probe page");

    let options = OpenOptions::from_content_dir(page_dir.to_string_lossy().to_string(), "WAE DPI probe");
    let mut backend = Win32Backend::open(options, Box::new(PrintHandler)).expect("open probe window");

    let started = Instant::now();
    while backend.pump() {
        if started.elapsed() >= Duration::from_secs(seconds) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = backend.close();
    eprintln!("[wae-dpi-probe] done elapsed_ms={}", started.elapsed().as_millis());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("dpi_probe is Windows only");
    std::process::exit(2);
}

#[cfg(windows)]
const PROBE_HTML: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>WAE DPI probe</title>
<style>html,body{margin:0;padding:0;background:#fff}</style></head>
<body>
<canvas id="probe" style="display:block;width:200px;height:120px"></canvas>
<div id="info" style="font:12px/1.4 Consolas,monospace;padding:8px;color:#000"></div>
<script>
(function () {
  var canvas = document.getElementById('probe');
  var info = document.getElementById('info');
  var state = { dpr: window.devicePixelRatio || 1, lineDeviceX: 0 };

  function draw() {
    var dpr = state.dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(200 * dpr);
    canvas.height = Math.round(120 * dpr);
    var ctx = canvas.getContext('2d');
    ctx.fillStyle = '#fff';
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.fillStyle = '#000';
    state.lineDeviceX = Math.round(40 * dpr);
    // 恰好 1 个画布（设备）像素宽，任何缩放/位图拉伸都会让它变宽或变灰。
    ctx.fillRect(state.lineDeviceX, 0, 1, canvas.height);
    info.textContent = 'dpr=' + dpr + ' backing=' + canvas.width + 'x' + canvas.height
      + ' line_device_x=' + state.lineDeviceX + ' inner=' + window.innerWidth + 'x' + window.innerHeight;
    report('draw');
  }

  function report(tag) {
    if (window.chrome && window.chrome.webview) {
      window.chrome.webview.postMessage(JSON.stringify({
        tag: tag,
        dpr: window.devicePixelRatio,
        lineDeviceX: state.lineDeviceX,
        backing: [canvas.width, canvas.height],
        inner: [window.innerWidth, window.innerHeight],
        screen: [screen.width, screen.height, screen.availWidth, screen.availHeight]
      }));
    }
  }

  window.addEventListener('resize', draw);
  draw();
})();
</script>
</body></html>
"#;
