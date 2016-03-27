# `wae-webview-win32`

自研 Win32 + WebView2 宿主，提供非阻塞的 [`WebViewHost::pump`](src/lib.rs)。

经门面 feature `webview` 时以 `wae::webview` 导出。

GUI 仅 Windows 可用；其他目标上 `WebViewHost::open` 返回不支持错误。
