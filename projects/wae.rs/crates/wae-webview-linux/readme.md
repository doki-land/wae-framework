# `wae-webview-linux`

Linux `WebKitGTK 4.1` + `GTK3` 后端，实现 `wae-webview` 的 `WebViewBackend`（`LinuxBackend`）。

能力面：`GtkWindow` + `WebKitWebView` 窗口、`content_dir`（本地文件 URL）/ `url` 加载、
`window.webkit.messageHandlers.wae` 入站消息、`window.__waeReceive` 出站回投、
`gtk_main_iteration_do` 驱动的非阻塞 `pump`、关闭。

GTK / WebKit 通过 `libloading` 在运行时解析：缺少 `libgtk-3`、`libwebkit2gtk-4.1` 或
`libjavascriptcoregtk-4.1` 时 `open` 返回带库名与符号名的明确错误，不静默降级。
`open` 在当前线程初始化 GTK，`pump` / `post_message` 必须在该线程调用。
非 Linux 目标上本 crate 不提供能力，也不会被门面选中。
