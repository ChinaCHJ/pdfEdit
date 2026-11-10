mod render;

use axum::{
    extract::DefaultBodyLimit,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use rust_embed::RustEmbed;
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;

const BIND_ADDR: &str = "127.0.0.1:17800";
const URL: &str = "http://127.0.0.1:17800";

/// 页面底图是 base64 内嵌的，十几页的文档很容易超过默认 2MB 上限
const MAX_BODY: usize = 256 * 1024 * 1024;

/// 前端资源编进二进制。
/// 发布时只有一个 exe，双击即用 —— 用户不用管 static 目录在哪、也不用装任何运行时。
#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

#[derive(Deserialize)]
struct ExportReq {
    pages: Vec<render::PageData>,
    #[serde(default)]
    title: String,
}

struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let msg = format!("{:#}", self.0);
        eprintln!("[error] {msg}");
        (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(e: E) -> Self {
        AppError(e.into())
    }
}

async fn export_handler(Json(req): Json<ExportReq>) -> Result<Response, AppError> {
    let page_count = req.pages.len();
    let bytes = tokio::task::spawn_blocking(move || render::pages_to_pdf(&req.pages, &req.title))
        .await
        .map_err(|e| anyhow::anyhow!("导出任务异常：{e}"))??;

    println!("导出完成：{page_count} 页，{} KB", bytes.len() / 1024);

    let disposition = "attachment; filename=\"edited.pdf\"";

    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf"),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        bytes,
    )
        .into_response())
}

/// 静态文件的 MIME。必须自己列，不能靠猜：
/// `.mjs` 认成 octet-stream 的话，浏览器会拒绝把它当 ES module 加载，整个前端直接白屏。
fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        // bcmap（中文 CMap）和 pfb（Type1 字体）都是二进制，pdf.js 只当字节流读
        _ => "application/octet-stream",
    }
}

fn respond(bytes: Vec<u8>, path: &str) -> Response {
    let mut res = bytes.into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(mime_of(path)),
    );
    res
}

/// exe 旁边如果放了 static 目录，就用它。开发时改前端不用重新编译，也给想换界面的人留了口子。
fn static_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("static");
    dir.join("index.html").is_file().then_some(dir)
}

async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // 拒绝路径穿越
    if path.contains("..") {
        return (StatusCode::NOT_FOUND, "404").into_response();
    }

    if let Some(dir) = static_dir() {
        let file = dir.join(path);
        if file.is_file() {
            if let Ok(bytes) = std::fs::read(&file) {
                return respond(bytes, path);
            }
        }
    }

    match Assets::get(path) {
        Some(f) => respond(f.data.into_owned(), path),
        None => (StatusCode::NOT_FOUND, "404").into_response(),
    }
}

fn open_browser(url: &str) {
    if std::env::var("PDF_EDITOR_NO_OPEN").is_ok() {
        return;
    }
    let _ = {
        #[cfg(target_os = "windows")]
        {
            // 第一个空参数是窗口标题，不能省，否则 start 会把 url 当标题
            Command::new("cmd").args(["/C", "start", "", url]).spawn()
        }
        #[cfg(target_os = "macos")]
        {
            Command::new("open").arg(url).spawn()
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            Command::new("xdg-open").arg(url).spawn()
        }
    };
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("PDF 编辑器 v{}", env!("CARGO_PKG_VERSION"));

    if let Some(dir) = static_dir() {
        println!("前端资源：外部目录 {}", dir.display());
    } else {
        println!("前端资源：内嵌（单文件模式）");
    }
    match render::find_browser() {
        Some(p) => println!("导出引擎：{}", p.display()),
        None => println!("导出引擎：未找到 Chrome / Edge —— 编辑能用，导出会失败"),
    }

    let listener = match tokio::net::TcpListener::bind(BIND_ADDR).await {
        Ok(l) => l,
        Err(e) => {
            println!("\n端口 {BIND_ADDR} 已被占用（{e}）。");
            println!("如果程序已经在运行，直接打开 {URL} 即可；否则关掉占用的程序再试。\n");
            open_browser(URL);
            return Ok(());
        }
    };

    let app = Router::new()
        .route("/api/export", post(export_handler))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .fallback(static_handler);

    println!("\n  已启动 → {URL}");
    println!("  关闭这个窗口即退出程序\n");

    open_browser(URL);
    axum::serve(listener, app).await?;
    Ok(())
}
