# `wae-webview-win32`

自研 Win32 + WebView2 宿主，提供非阻塞 [`pump`](src/lib.rs) API，便于在 Node / 薄 N-API 事件循环里驱动原生窗口。

## 内容加载

优先使用 WebView2 虚拟主机映射：`https://app.local/` → 本地内容目录，避免 Unicode 路径下 `file://` 导航失败。

## 平台

仅 Windows。其他目标返回 [`WebViewError::Unsupported`](src/error.rs)。
