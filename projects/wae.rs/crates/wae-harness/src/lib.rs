//! WAE 自动化 harness：在进程内驱动一个真实 WebView 宿主。
//!
//! 只依赖 [`wae_webview`] 合同，不依赖具体平台后端；平台能力由后端实现，
//! 缺能力时沿用 `WebViewError::Unsupported`，本 crate 不伪造成功。
//!
//! 所有等待都驱动 `WebViewHost::pump()`，不在其它线程碰窗口，也不忙等。

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use wae_webview::{
    automation_codes, AutomationId, CaptureOutcome, IpcHandler, OpenOptions, Result as WebViewResult, ScriptOutcome, WebViewBackend,
    WebViewError, WebViewHost,
};

/// 单次操作默认超时。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// 轮询间隔；避免把窗口消息循环打满。
const POLL_INTERVAL: Duration = Duration::from_millis(5);
/// 单次 `wait_for` 求值的最长等待；避免一次求值吃掉整个超时预算。
const EVAL_SLICE: Duration = Duration::from_millis(500);

/// harness 失败分类；`code()` 是稳定字符串。
#[derive(Debug)]
pub enum HarnessError {
    /// 宿主层失败或平台缺少能力。
    WebView(WebViewError),
    /// 窗口已销毁。
    WindowClosed,
    /// 页面脚本抛错。
    Script {
        /// 稳定错误码，来自后端结果。
        code: &'static str,
        /// 异常文本。
        message: String,
    },
    /// 截图失败。
    Capture {
        /// 稳定错误码，来自后端结果。
        code: &'static str,
        /// 失败说明。
        message: String,
    },
    /// 等待超时；`last` 是最后一次求值结果。
    Timeout {
        /// 正在等待的脚本表达式。
        script: String,
        /// 最后一次结果。
        last: serde_json::Value,
    },
}

impl HarnessError {
    /// 稳定错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::WebView(WebViewError::Unsupported { .. }) => "harness_unsupported",
            Self::WebView(_) => "harness_webview",
            Self::WindowClosed => automation_codes::WINDOW_CLOSED,
            Self::Script { code, .. } | Self::Capture { code, .. } => code,
            Self::Timeout { .. } => "harness_timeout",
        }
    }
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WebView(error) => write!(formatter, "webview: {error}"),
            Self::WindowClosed => write!(formatter, "window closed"),
            Self::Script { code, message } => write!(formatter, "script {code}: {message}"),
            Self::Capture { code, message } => write!(formatter, "capture {code}: {message}"),
            Self::Timeout { script, last } => write!(formatter, "timeout waiting for `{script}` (last: {last})"),
        }
    }
}

impl std::error::Error for HarnessError {}

impl From<WebViewError> for HarnessError {
    fn from(error: WebViewError) -> Self {
        match &error {
            WebViewError::Host(failure) if failure.code == automation_codes::WINDOW_CLOSED => Self::WindowClosed,
            _ => Self::WebView(error),
        }
    }
}

/// harness 结果别名。
pub type Result<T> = std::result::Result<T, HarnessError>;

/// 进程内驱动一个 WebView 宿主。
pub struct Harness<B: WebViewBackend> {
    host: WebViewHost<B>,
}

impl<B: WebViewBackend> Harness<B> {
    /// 打开宿主；隐藏窗口由 `options.hidden` 决定。
    pub fn open(options: OpenOptions, handler: Box<dyn IpcHandler>) -> Result<Self> {
        Ok(Self { host: WebViewHost::open(options, handler)? })
    }

    /// 宿主只读访问（例如查询 `backend_name`）。
    pub fn host(&self) -> &WebViewHost<B> {
        &self.host
    }

    /// 宿主可变访问（例如投递 IPC 或 `focus`）。
    pub fn host_mut(&mut self) -> &mut WebViewHost<B> {
        &mut self.host
    }

    /// 窗口是否仍打开。
    pub fn is_alive(&self) -> bool {
        self.host.is_alive()
    }

    /// 等待页面完成加载。
    pub fn ready(&mut self, timeout: Duration) -> Result<()> {
        self.wait_for("document.readyState === 'complete'", timeout).map(|_| ())
    }

    /// 执行脚本并返回 JSON 结果。
    pub fn eval(&mut self, script: &str) -> Result<serde_json::Value> {
        self.eval_within(script, Instant::now() + DEFAULT_TIMEOUT)
    }

    /// 执行脚本并返回 JSON 结果，使用自定义超时。
    pub fn eval_timeout(&mut self, script: &str, timeout: Duration) -> Result<serde_json::Value> {
        self.eval_within(script, Instant::now() + timeout)
    }

    /// 轮询表达式直到结果为 `true`；超时错误携带最后一次结果。
    pub fn wait_for(&mut self, script: &str, timeout: Duration) -> Result<serde_json::Value> {
        let deadline = Instant::now() + timeout;
        loop {
            let slice = (Instant::now() + EVAL_SLICE).min(deadline);
            let value = self.eval_within(script, slice)?;
            if value == serde_json::Value::Bool(true) {
                return Ok(value);
            }
            if Instant::now() >= deadline {
                return Err(HarnessError::Timeout { script: script.to_owned(), last: value });
            }
            self.pump_once()?;
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// 截图写入 `path`（父目录自动创建），返回实际路径。
    pub fn screenshot(&mut self, path: &Path) -> Result<PathBuf> {
        self.screenshot_timeout(path, DEFAULT_TIMEOUT)
    }

    /// 截图并指定超时。
    pub fn screenshot_timeout(&mut self, path: &Path, timeout: Duration) -> Result<PathBuf> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| HarnessError::Capture {
                code: automation_codes::CAPTURE_FAILED,
                message: format!("cannot create screenshot directory {}: {error}", parent.display()),
            })?;
        }
        let id = self.host.screenshot(path)?;
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(outcome) = self.host.poll_capture_result(id) {
                return finish_capture(outcome);
            }
            if Instant::now() >= deadline {
                return Err(HarnessError::Timeout {
                    script: format!("screenshot({})", path.display()),
                    last: serde_json::Value::Null,
                });
            }
            self.pump_once()?;
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// 请求关闭窗口。
    pub fn close(&mut self) -> Result<()> {
        self.host.close()?;
        Ok(())
    }

    /// 执行一次脚本，最多等到 `deadline`。
    fn eval_within(&mut self, script: &str, deadline: Instant) -> Result<serde_json::Value> {
        let id = self.host.evaluate(script)?;
        let outcome = self.wait_script(id, deadline)?;
        match outcome.error {
            Some(error) => Err(HarnessError::Script { code: error.code, message: error.message }),
            None => Ok(outcome.value.unwrap_or(serde_json::Value::Null)),
        }
    }

    /// 等待指定脚本任务完成。
    fn wait_script(&mut self, id: AutomationId, deadline: Instant) -> Result<ScriptOutcome> {
        loop {
            if let Some(outcome) = self.host.poll_script_result(id) {
                return Ok(outcome);
            }
            if Instant::now() >= deadline {
                return Err(HarnessError::Timeout {
                    script: format!("script#{id}"),
                    last: serde_json::Value::Null,
                });
            }
            self.pump_once()?;
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// 驱动一次消息泵；窗口已销毁返回 `WindowClosed`。
    fn pump_once(&mut self) -> Result<()> {
        if self.host.pump() {
            Ok(())
        }
        else {
            Err(HarnessError::WindowClosed)
        }
    }
}

fn finish_capture(outcome: CaptureOutcome) -> Result<PathBuf> {
    match (outcome.path, outcome.error) {
        (Some(path), _) => Ok(path),
        (None, Some(error)) => Err(HarnessError::Capture { code: error.code, message: error.message }),
        (None, None) => Err(HarnessError::Capture {
            code: automation_codes::CAPTURE_FAILED,
            message: "capture completed without a file".into(),
        }),
    }
}

/// 一条控制通道请求。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControlRequest {
    id: u64,
    op: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    script: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

/// 一条控制通道响应。
#[derive(Debug, serde::Serialize)]
struct ControlResponse {
    id: u64,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl ControlResponse {
    fn ok(id: u64, value: serde_json::Value) -> Self {
        Self { id, ok: true, value: Some(value), code: None, message: None }
    }

    fn failed(id: u64, error: &HarnessError) -> Self {
        Self { id, ok: false, value: None, code: Some(error.code().to_owned()), message: Some(error.to_string()) }
    }
}

/// 在给定连接上处理控制请求，直到客户端关闭或发出 `close`。
pub fn serve_connection<B: WebViewBackend>(
    harness: &mut Harness<B>,
    stream: std::net::TcpStream,
    token: Option<&str>,
) -> std::io::Result<()> {
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }
        let request: ControlRequest = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                write_response(&mut writer, &ControlResponse {
                    id: 0,
                    ok: false,
                    value: None,
                    code: Some("harness_bad_request".into()),
                    message: Some(error.to_string()),
                })?;
                continue;
            }
        };
        let id = request.id;
        if let Some(expected) = token
            && request.token.as_deref() != Some(expected)
        {
            write_response(&mut writer, &ControlResponse {
                id,
                ok: false,
                value: None,
                code: Some("harness_unauthorized".into()),
                message: Some("control token mismatch".into()),
            })?;
            continue;
        }
        let timeout = Duration::from_millis(request.timeout_ms.unwrap_or(10_000));
        let (response, closing) = match request.op.as_str() {
            "ready" => match harness.ready(timeout) {
                Ok(()) => (ControlResponse::ok(id, serde_json::Value::Bool(true)), false),
                Err(error) => (ControlResponse::failed(id, &error), false),
            },
            "eval" => match request.script.as_deref() {
                Some(script) => match harness.eval_timeout(script, timeout) {
                    Ok(value) => (ControlResponse::ok(id, value), false),
                    Err(error) => (ControlResponse::failed(id, &error), false),
                },
                None => (bad_request(id, "`script` is required"), false),
            },
            "waitFor" => match request.script.as_deref() {
                Some(script) => match harness.wait_for(script, timeout) {
                    Ok(value) => (ControlResponse::ok(id, value), false),
                    Err(error) => (ControlResponse::failed(id, &error), false),
                },
                None => (bad_request(id, "`script` is required"), false),
            },
            "screenshot" => match request.path.as_deref() {
                Some(path) => match harness.screenshot_timeout(Path::new(path), timeout) {
                    Ok(path) => (ControlResponse::ok(id, serde_json::Value::String(path.display().to_string())), false),
                    Err(error) => (ControlResponse::failed(id, &error), false),
                },
                None => (bad_request(id, "`path` is required"), false),
            },
            "close" => match harness.close() {
                Ok(()) => (ControlResponse::ok(id, serde_json::Value::Bool(true)), true),
                Err(error) => (ControlResponse::failed(id, &error), true),
            },
            other => (bad_request(id, &format!("unknown op `{other}`")), false),
        };
        write_response(&mut writer, &response)?;
        if closing {
            return Ok(());
        }
    }
}

fn bad_request(id: u64, message: &str) -> ControlResponse {
    ControlResponse {
        id,
        ok: false,
        value: None,
        code: Some("harness_bad_request".into()),
        message: Some(message.to_owned()),
    }
}

fn write_response(writer: &mut std::net::TcpStream, response: &ControlResponse) -> std::io::Result<()> {
    let line = serde_json::to_string(response).map_err(std::io::Error::other)?;
    writer.write_all(line.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

/// 环境变量激活：`WAE_HARNESS_PORT` 存在时在 loopback 上服务一个控制客户端。
///
/// 未设置时立即返回 `Ok(false)`；只绑定 `127.0.0.1`，不接受任何非 loopback 地址。
pub fn serve_from_env<B: WebViewBackend>(harness: &mut Harness<B>) -> std::io::Result<bool> {
    let Some(port) = std::env::var("WAE_HARNESS_PORT").ok().and_then(|raw| raw.parse::<u16>().ok())
    else {
        return Ok(false);
    };
    let token = std::env::var("WAE_HARNESS_TOKEN").ok().filter(|value| !value.is_empty());
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    println!("wae-harness listening on 127.0.0.1:{port}");
    let (stream, _) = listener.accept()?;
    serve_connection(harness, stream, token.as_deref())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use wae_webview::{IpcOutcome, IpcSink, Platform};

    /// 假后端：记录脚本，第 `complete_after` 次 pump 后产出结果。
    struct FakeBackend {
        alive: bool,
        scripts: Arc<Mutex<Vec<(AutomationId, String)>>>,
        results: Vec<ScriptOutcome>,
        pumps: usize,
        complete_after: usize,
        reply: Option<serde_json::Value>,
    }

    impl WebViewBackend for FakeBackend {
        const NAME: &'static str = "fake";
        const PLATFORM: Platform = Platform::Windows;

        fn open(_options: OpenOptions, _handler: Box<dyn IpcHandler>) -> WebViewResult<Self> {
            Ok(Self {
                alive: true,
                scripts: Arc::new(Mutex::new(Vec::new())),
                results: Vec::new(),
                pumps: 0,
                complete_after: 2,
                reply: Some(serde_json::Value::Bool(true)),
            })
        }

        fn pump(&mut self) -> bool {
            self.pumps += 1;
            if self.pumps % self.complete_after == 0
                && let Some((id, script)) = self.scripts.lock().unwrap().pop()
            {
                let value = if script.contains("'pending'") {
                    serde_json::Value::Bool(false)
                }
                else {
                    self.reply.clone().unwrap_or(serde_json::Value::Null)
                };
                self.results.push(ScriptOutcome::success(id, value));
            }
            self.alive
        }

        fn close(&mut self) -> WebViewResult<()> {
            self.alive = false;
            Ok(())
        }

        fn post_message(&self, _json: &str) -> WebViewResult<()> {
            Ok(())
        }

        fn is_alive(&self) -> bool {
            self.alive
        }

        fn focus(&mut self) -> WebViewResult<()> {
            Ok(())
        }

        fn evaluate(&mut self, id: AutomationId, script: &str) -> WebViewResult<()> {
            self.scripts.lock().unwrap().push((id, script.to_owned()));
            Ok(())
        }

        fn take_script_result(&mut self, id: AutomationId) -> Option<ScriptOutcome> {
            let index = self.results.iter().position(|outcome| outcome.id == id)?;
            Some(self.results.remove(index))
        }
    }

    struct NoopHandler;

    impl IpcHandler for NoopHandler {
        fn on_ipc(&mut self, _message: &str, _sink: &IpcSink) -> IpcOutcome {
            IpcOutcome::Ignored
        }
    }

    fn harness() -> Harness<FakeBackend> {
        Harness::open(OpenOptions::from_url("https://example.com", "fake"), Box::new(NoopHandler)).expect("open fake backend")
    }

    #[test]
    fn eval_returns_the_page_value() {
        let mut harness = harness();
        let value = harness.eval("1 + 1").expect("eval");
        assert_eq!(value, serde_json::Value::Bool(true));
    }

    #[test]
    fn wait_for_times_out_with_last_value() {
        let mut harness = harness();
        let error = harness.wait_for("'pending'", Duration::from_millis(60)).expect_err("must time out");
        assert_eq!(error.code(), "harness_timeout");
        assert!(error.to_string().contains("false"), "unexpected error: {error}");
    }

    #[test]
    fn closed_window_reports_window_closed() {
        let mut harness = harness();
        harness.close().expect("close");
        let error = harness.eval("1").expect_err("closed window");
        assert_eq!(error.code(), automation_codes::WINDOW_CLOSED);
    }
}
