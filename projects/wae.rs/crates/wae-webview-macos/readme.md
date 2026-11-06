# `wae-webview-macos`

macOS `WKWebView` + `NSWindow` 后端（objc2 系），实现 `wae-webview` 的 `WebViewBackend`（`MacosBackend`）。

能力面：窗口、`content_dir` / `url` 加载、`window.webkit.messageHandlers.wae` 入站消息、
`window.__waeReceive` 出站回投、非阻塞 `pump`、关闭。

约束：必须在主线程打开，否则 `open` 返回明确错误。非 macOS 目标上本 crate 不提供能力，
也不会被门面选中。
