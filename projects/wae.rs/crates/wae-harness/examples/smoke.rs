//! 隐藏窗口烟雾测试：真实 WebView2 上执行脚本、等待条件并截图。
//!
//! 运行：`cargo run -p wae-harness --example smoke`

#[cfg(windows)]
fn main() {
    use std::time::Duration;

    use wae_harness::Harness;
    use wae_webview::OpenOptions;
    use wae_webview_win32::Win32Backend;

    struct Noop;
    impl wae_webview::IpcHandler for Noop {
        fn on_ipc(&mut self, _message: &str, _sink: &wae_webview::IpcSink) -> wae_webview::IpcOutcome {
            wae_webview::IpcOutcome::Ignored
        }
    }

    let options = OpenOptions::from_url("data:text/html,<title>smoke</title><h1 id=title>harness</h1>", "wae-harness smoke").with_hidden(true);
    let mut harness = match Harness::<Win32Backend>::open(options, Box::new(Noop)) {
        Ok(harness) => harness,
        Err(error) => {
            eprintln!("open failed: {error}");
            std::process::exit(1);
        }
    };

    let result = (|| -> wae_harness::Result<()> {
        harness.ready(Duration::from_secs(10))?;
        let title = harness.eval("document.querySelector('#title').textContent")?;
        println!("eval title = {title}");
        assert_eq!(title, serde_json::Value::String("harness".into()));
        harness.wait_for("document.querySelector('#title').textContent === 'harness'", Duration::from_secs(5))?;
        let shot = std::env::temp_dir().join("wae-harness-smoke.png");
        let written = harness.screenshot(&shot)?;
        let size = std::fs::metadata(&written).map(|meta| meta.len()).unwrap_or(0);
        println!("screenshot {} bytes = {size}", written.display());
        assert!(size > 0, "screenshot is empty");
        harness.close()?;
        Ok(())
    })();

    match result {
        Ok(()) => println!("harness smoke ok"),
        Err(error) => {
            eprintln!("harness smoke failed: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("harness smoke is only wired for Windows");
}
