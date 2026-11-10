//! 把「原页面底图 + 覆盖层」拼成 PDF。
//!
//! 覆盖层有三类来源，最终都归一到 `Overlay` 上：
//!   1. 用户改过的文字行
//!   2. 批注标注（高亮、下划线、框、箭头、便签）
//!   3. 表单字段的填写值
//!
//! 未改动的部分直接就是原页面的高分辨率位图，像素级还原。
//! 排版和中文断行仍然交给本机 Edge / Chrome 的打印引擎。

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/* ---------------- 数据模型 ---------------- */

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageData {
    pub width: f64,
    pub height: f64,
    /// data:image/jpeg;base64,... 形式的页面底图
    pub image: String,
    #[serde(default)]
    pub overlays: Vec<Overlay>,
    #[serde(default)]
    pub watermark: Option<Watermark>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watermark {
    pub text: String,
    #[serde(default = "d_fs")]
    pub font_size: f64,
    #[serde(default = "d_op")]
    pub opacity: f64,
    #[serde(default = "d_angle")]
    pub angle: f64,
    #[serde(default = "d_ink")]
    pub color: String,
}

fn d_fs() -> f64 {
    26.0
}
fn d_font() -> String {
    "'Microsoft YaHei','微软雅黑',sans-serif".into()
}
fn d_op() -> f64 {
    0.14
}
fn d_angle() -> f64 {
    -30.0
}
fn d_ink() -> String {
    "#888888".into()
}
fn d_white() -> String {
    "#ffffff".into()
}
fn d_black() -> String {
    "#111111".into()
}
fn d_1() -> f64 {
    1.0
}
fn d_2() -> f64 {
    2.0
}

/// 覆盖层元素。`kind` 决定形状，坐标全部是页面 CSS px。
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Overlay {
    /// 文字（被改过的原文行、表单填写的文本值都走这里）
    #[serde(rename_all = "camelCase")]
    Text {
        left: f64,
        top: f64,
        width: f64,
        height: f64,
        font_size: f64,
        text: String,
        #[serde(default = "d_white")]
        bg: String,
        #[serde(default = "d_black")]
        color: String,
        #[serde(default)]
        bold: bool,
        /// 为 true 时不画底色（表单值直接压在原始空白格上）
        #[serde(default)]
        transparent_bg: bool,
        /// CSS font-family，前端传单引号形式，便于直接塞进 style 属性
        #[serde(default = "d_font")]
        font: String,
        /// 首行缩进（px）。合并整段后左边界会撑到最左侧，首行缩进改由 text-indent 表达
        #[serde(default)]
        indent: f64,
        /// 原始行距（px）。为 0 时按字号推算
        #[serde(default)]
        line_gap: f64,
    },
    /// 高亮（半透明色块）
    #[serde(rename_all = "camelCase")]
    Highlight {
        left: f64,
        top: f64,
        width: f64,
        height: f64,
        #[serde(default = "d_ink")]
        color: String,
        #[serde(default = "d_op")]
        opacity: f64,
    },
    /// 直线（下划线 / 删除线）
    #[serde(rename_all = "camelCase")]
    Rule {
        left: f64,
        top: f64,
        width: f64,
        #[serde(default = "d_1")]
        thickness: f64,
        #[serde(default = "d_black")]
        color: String,
    },
    /// 矩形框
    #[serde(rename_all = "camelCase")]
    Box {
        left: f64,
        top: f64,
        width: f64,
        height: f64,
        #[serde(default = "d_ink")]
        color: String,
        #[serde(default = "d_2")]
        thickness: f64,
        #[serde(default)]
        dashed: bool,
    },
    /// 椭圆圈选
    #[serde(rename_all = "camelCase")]
    Ellipse {
        left: f64,
        top: f64,
        width: f64,
        height: f64,
        #[serde(default = "d_ink")]
        color: String,
        #[serde(default = "d_2")]
        thickness: f64,
    },
    /// 箭头
    #[serde(rename_all = "camelCase")]
    Arrow {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        #[serde(default = "d_ink")]
        color: String,
        #[serde(default = "d_2")]
        thickness: f64,
    },
    /// 便签（右下角折角的文字块）
    #[serde(rename_all = "camelCase")]
    Note {
        left: f64,
        top: f64,
        width: f64,
        height: f64,
        text: String,
        #[serde(default = "d_ink")]
        color: String,
        #[serde(default = "d_fs_small")]
        font_size: f64,
    },
}

fn d_fs_small() -> f64 {
    12.0
}

/* ---------------- 渲染 ---------------- */

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("<br>"),
            _ => out.push(c),
        }
    }
    out
}

fn n(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() { "0".into() } else { s }
}

/// SVG 图形类覆盖层
fn svg_shape(o: &Overlay) -> Option<String> {
    match o {
        Overlay::Rule {
            left,
            top,
            width,
            thickness,
            color,
        } => Some(format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
            n(*left),
            n(*top),
            n(*width),
            n(thickness.max(0.5)),
            esc(color)
        )),
        Overlay::Box {
            left,
            top,
            width,
            height,
            color,
            thickness,
            dashed,
        } => {
            let dash = if *dashed {
                format!(" stroke-dasharray=\"{} {}\"", n(thickness * 3.0), n(thickness * 2.0))
            } else {
                String::new()
            };
            Some(format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"{}/>",
                n(*left),
                n(*top),
                n(*width),
                n(*height),
                esc(color),
                n(*thickness),
                dash
            ))
        }
        Overlay::Ellipse {
            left,
            top,
            width,
            height,
            color,
            thickness,
        } => Some(format!(
            "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"/>",
            n(left + width / 2.0),
            n(top + height / 2.0),
            n((width / 2.0).max(0.5)),
            n((height / 2.0).max(0.5)),
            esc(color),
            n(*thickness)
        )),
        Overlay::Arrow {
            x1,
            y1,
            x2,
            y2,
            color,
            thickness,
        } => Some(format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"{}\" marker-end=\"url(#ahd)\"/>",
            n(*x1),
            n(*y1),
            n(*x2),
            n(*y2),
            esc(color),
            n(*thickness)
        )),
        Overlay::Highlight {
            left,
            top,
            width,
            height,
            color,
            opacity,
        } => Some(format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\" fill-opacity=\"{}\"/>",
            n(*left),
            n(*top),
            n(*width),
            n(*height),
            esc(color),
            n(*opacity)
        )),
        _ => None,
    }
}

/// div 类覆盖层（文字、便签）
fn html_div(o: &Overlay) -> Option<String> {
    match o {
        Overlay::Text {
            left,
            top,
            width,
            height,
            font_size,
            text,
            bg,
            color,
            bold,
            transparent_bg,
            font,
            indent,
            line_gap,
        } => {
            let weight = if *bold { "700" } else { "400" };
            let background = if *transparent_bg {
                "transparent".to_string()
            } else {
                bg.clone()
            };
            // 单行替换要让文字精确落在原行位置，行高就取字号；
            // 合并成多行的块则用前端量到的原始行距，没有就按字号估。
            let line_h = if *line_gap > 0.0 {
                *line_gap
            } else if *height > font_size * 1.6 {
                font_size * 1.5
            } else {
                *font_size
            };
            let indent_css = if *indent > 0.0 {
                format!("text-indent:{}px;", n(*indent))
            } else {
                String::new()
            };
            // 单行替换用 min-width：原文行宽刚好装下原字数，锁死宽度会让多一个字就折行。
            // 合并成整段的块反过来必须锁宽度，否则重排后的文字会一路排到页面外。
            let box_css = if *line_gap > 0.0 {
                format!("width:{}px;min-height:{}px;", n(*width), n(*height))
            } else {
                format!("min-width:{}px;min-height:{}px;", n(*width), n(*height))
            };
            Some(format!(
                "<div class=\"ov\" style=\"left:{}px;top:{}px;{}\
                 font-size:{}px;line-height:{}px;background:{};color:{};font-weight:{};\
                 font-family:{};{}\">{}</div>",
                n(*left),
                n(*top),
                box_css,
                n(*font_size),
                n(line_h),
                background,
                esc(color),
                weight,
                font,
                indent_css,
                esc(text)
            ))
        }
        Overlay::Note {
            left,
            top,
            width,
            height,
            text,
            color,
            font_size,
        } => Some(format!(
            "<div class=\"note\" style=\"left:{}px;top:{}px;width:{}px;min-height:{}px;\
             border-color:{};font-size:{}px\">{}</div>",
            n(*left),
            n(*top),
            n(*width),
            n(*height),
            esc(color),
            n(*font_size),
            esc(text)
        )),
        _ => None,
    }
}

fn render_watermark(wm: &Watermark, w: f64, h: f64) -> String {
    let fs = wm.font_size.max(6.0);
    let text = esc(&wm.text);
    // 用字宽估算平铺间距，中文按 1em 算
    let step_x = fs * (wm.text.chars().count() as f64 * 1.05 + 6.0);
    let step_y = fs * 4.2;

    let rows = ((h / step_y).ceil() as i32 + 4).max(1);
    let cols = ((w / step_x).ceil() as i32 + 4).max(1);

    let mut cells = String::new();
    for r in -2..rows - 2 {
        for c in -2..cols - 2 {
            let x = c as f64 * step_x;
            let y = (r as f64 + 0.5) * step_y;
            cells.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" font-size=\"{}\" fill=\"{}\">{}</text>",
                n(x),
                n(y),
                n(fs),
                esc(&wm.color),
                text
            ));
        }
    }

    format!(
        "<svg class=\"wm\" viewBox=\"0 0 {} {}\" preserveAspectRatio=\"none\" \
         style=\"position:absolute;left:0;top:0;width:100%;height:100%;opacity:{}\">\
         <g transform=\"rotate({} {} {})\" font-family=\"Microsoft YaHei,sans-serif\">{}</g></svg>",
        n(w),
        n(h),
        n(wm.opacity),
        n(wm.angle),
        n(w / 2.0),
        n(h / 2.0),
        cells
    )
}

const BASE_CSS: &str = r#"
* { margin: 0; padding: 0; box-sizing: border-box; }
html, body { background: #fff; }
.page {
  position: relative;
  overflow: hidden;
  page-break-after: always;
  break-after: page;
}
.page:last-child { page-break-after: auto; break-after: auto; }
.bg {
  position: absolute;
  left: 0; top: 0;
  width: 100%; height: 100%;
  display: block;
}
.shapes {
  position: absolute;
  left: 0; top: 0;
  width: 100%; height: 100%;
  overflow: visible;
}
.ov {
  position: absolute;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: "Microsoft YaHei", "微软雅黑", "PingFang SC", "SimSun", sans-serif;
  text-indent: 0;
}
.note {
  position: absolute;
  padding: 4px 6px;
  background: #fffbe6;
  border: 1px solid #d9a441;
  border-radius: 3px;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: "Microsoft YaHei", sans-serif;
  line-height: 1.45;
  color: #333;
}
"#;

fn build_document(pages: &[PageData], title: &str) -> String {
    // 同一份文档里可能出现不同尺寸的页（比如旋转过某一页），
    // 所以给每种尺寸生成一条命名 @page 规则，各页用自己的尺寸打印。
    let mut keys: Vec<String> = Vec::new();
    let mut sizes: Vec<(String, String)> = Vec::new();
    for p in pages {
        let key = format!("{}x{}", n(p.width), n(p.height));
        if !keys.contains(&key) {
            keys.push(key);
            sizes.push((n(p.width), n(p.height)));
        }
    }

    let mut page_css = String::new();
    for (i, (w, h)) in sizes.iter().enumerate() {
        page_css.push_str(&format!(
            "@page pg{i} {{ size: {w}px {h}px; margin: 0; }}\n.pg{i} {{ page: pg{i}; }}\n"
        ));
    }

    let mut body = String::new();

    for page in pages {
        let key = format!("{}x{}", n(page.width), n(page.height));
        let slot = keys.iter().position(|k| *k == key).unwrap_or(0);

        body.push_str(&format!(
            "<div class=\"page pg{}\" style=\"width:{}px;height:{}px\">\n",
            slot,
            n(page.width),
            n(page.height)
        ));
        body.push_str(&format!("<img class=\"bg\" src=\"{}\">\n", page.image));

        // 图形类
        let shapes: String = page.overlays.iter().filter_map(svg_shape).collect();
        if !shapes.is_empty() {
            body.push_str(&format!(
                "<svg class=\"shapes\" viewBox=\"0 0 {} {}\" preserveAspectRatio=\"none\">\
                 <defs><marker id=\"ahd\" viewBox=\"0 0 10 10\" refX=\"8\" refY=\"5\" \
                 markerWidth=\"5\" markerHeight=\"5\" orient=\"auto-start-reverse\">\
                 <path d=\"M1 1L8 5L1 9\" fill=\"none\" stroke=\"#333\" stroke-width=\"1.6\" \
                 stroke-linecap=\"round\" stroke-linejoin=\"round\"/></marker></defs>{}</svg>\n",
                n(page.width),
                n(page.height),
                shapes
            ));
        }

        // 文字类
        for o in &page.overlays {
            if let Some(html) = html_div(o) {
                body.push_str(&html);
                body.push('\n');
            }
        }

        // 水印压在最上层
        if let Some(wm) = &page.watermark {
            if !wm.text.trim().is_empty() {
                body.push_str(&render_watermark(wm, page.width, page.height));
            }
        }

        body.push_str("</div>\n");
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<title>{}</title>
<style>
{}
{}
</style>
</head>
<body>
{}
</body>
</html>"#,
        esc(title),
        page_css,
        BASE_CSS,
        body
    )
}

/* ---------------- 浏览器调用 ---------------- */

pub fn find_browser() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    let pf = std::env::var("ProgramFiles").ok();
    let pf86 = std::env::var("ProgramFiles(x86)").ok();
    let local = std::env::var("LOCALAPPDATA").ok();

    for base in [pf86.as_deref(), pf.as_deref()].into_iter().flatten() {
        candidates.push(PathBuf::from(base).join(r"Microsoft\Edge\Application\msedge.exe"));
        candidates.push(PathBuf::from(base).join(r"Google\Chrome\Application\chrome.exe"));
    }
    if let Some(base) = local.as_deref() {
        candidates.push(PathBuf::from(base).join(r"Google\Chrome\Application\chrome.exe"));
        candidates.push(PathBuf::from(base).join(r"Microsoft\Edge\Application\msedge.exe"));
    }
    candidates.push(PathBuf::from("/usr/bin/google-chrome"));
    candidates.push(PathBuf::from("/usr/bin/chromium"));
    candidates.push(PathBuf::from(
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ));

    candidates.into_iter().find(|p| p.is_file())
}

fn to_file_url(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file:///");
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                out.push(*b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(windows)]
fn quiet(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(windows))]
fn quiet(cmd: &mut Command) -> &mut Command {
    cmd
}

pub fn pages_to_pdf(pages: &[PageData], title: &str) -> Result<Vec<u8>> {
    if pages.is_empty() {
        return Err(anyhow!("没有可导出的页面"));
    }

    let browser = find_browser().ok_or_else(|| {
        anyhow!(
            "没找到 Edge 或 Chrome，无法生成 PDF。\n\
             请安装 Microsoft Edge 或 Google Chrome 后重试。"
        )
    })?;

    let dir = tempfile::tempdir().context("创建临时目录失败")?;
    let html_path = dir.path().join("doc.html");
    let pdf_path = dir.path().join("out.pdf");
    let profile = dir.path().join("profile");

    std::fs::write(&html_path, build_document(pages, title)).context("写入临时 HTML 失败")?;

    let url = to_file_url(&html_path);
    let pdf_arg = format!("--print-to-pdf={}", pdf_path.display());
    let profile_arg = format!("--user-data-dir={}", profile.display());

    let mut last_err = String::new();

    for headless in ["--headless=new", "--headless"] {
        let _ = std::fs::remove_file(&pdf_path);

        let output = quiet(
            Command::new(&browser)
                .arg(headless)
                .arg("--disable-gpu")
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--disable-extensions")
                .arg("--no-pdf-header-footer")
                .arg("--virtual-time-budget=8000")
                .arg(&profile_arg)
                .arg(&pdf_arg)
                .arg(&url),
        )
        .output()
        .with_context(|| format!("启动浏览器失败：{}", browser.display()))?;

        if pdf_path.is_file() {
            let bytes = std::fs::read(&pdf_path).context("读取生成的 PDF 失败")?;
            if bytes.starts_with(b"%PDF") && bytes.len() > 800 {
                return Ok(bytes);
            }
            last_err = format!("生成的 PDF 不完整（{} 字节）", bytes.len());
            continue;
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        last_err = format!(
            "{headless} 退出码 {:?}，未产出文件。stderr: {}",
            output.status.code(),
            stderr.chars().take(400).collect::<String>()
        );
    }

    Err(anyhow!("PDF 渲染失败：{last_err}"))
}
