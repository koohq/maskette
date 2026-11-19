//! `index.html` next to the comparison's images: the runs side by side.
//!
//! Built from `results.jsonl` (the last run of each model and mode wins),
//! so it can be rebuilt at any time without calling a model.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::slug;

/// The modes in page order, with their headings.
const MODES: [(&str, &str); 5] = [
    ("front", "正面図"),
    ("turnaround", "四面図"),
    ("local", "部分再生成"),
    ("eyes", "目の拡大図"),
    ("eyes-art", "目の拡大図（元の絵の目つき）"),
];

/// Space kept around the masked area in the before/after crops (px on the
/// 800x1100 front view).
const CROP_MARGIN: u32 = 60;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The models in the order they first ran, and the last run of each
/// model and mode.
fn read_results(out: &Path) -> (Vec<String>, BTreeMap<(String, String), Value>) {
    let text = std::fs::read_to_string(out.join("results.jsonl")).unwrap_or_default();
    let mut models = Vec::new();
    let mut runs = BTreeMap::new();
    for record in text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        let (Some(model), Some(mode)) = (
            record.get("model").and_then(Value::as_str),
            record.get("mode").and_then(Value::as_str),
        ) else {
            continue;
        };
        if !models.iter().any(|known| known == model) {
            models.push(model.to_owned());
        }
        runs.insert((model.to_owned(), mode.to_owned()), record);
    }
    (models, runs)
}

fn cost(run: &Value) -> Option<f64> {
    run.get("cost_usd").and_then(Value::as_f64)
}

/// One run's numbers for the table, or why it failed.
fn summary(run: Option<&Value>) -> String {
    let Some(run) = run else {
        return "<span class=\"muted\">—</span>".to_owned();
    };
    if run.get("ok").and_then(Value::as_bool) != Some(true) {
        let error = run.get("error").and_then(Value::as_str).unwrap_or("");
        return format!(
            "<span class=\"failed\">失敗</span> <small>{}</small>",
            escape(error)
        );
    }
    let mut parts = Vec::new();
    if let Some(seconds) = run.get("seconds").and_then(Value::as_f64) {
        parts.push(format!("{seconds:.0} 秒"));
    }
    if let Some(cost) = cost(run) {
        parts.push(format!("${cost:.3}"));
    }
    if let Some([width, height]) = run
        .get("size")
        .and_then(Value::as_array)
        .and_then(|size| <[Value; 2]>::try_from(size.clone()).ok())
    {
        parts.push(format!("{width}×{height}"));
    }
    parts.join("<br>")
}

/// The masked area of a local revision, grown by a margin and kept inside
/// the front view.
fn crop_box(mask: &Path) -> Option<(u32, u32, u32, u32)> {
    let mask = image::open(mask).ok()?.to_luma8();
    let (width, height) = mask.dimensions();
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for (x, y, pixel) in mask.enumerate_pixels() {
        if pixel.0[0] > 127 {
            found = Some(found.map_or((x, y, x, y), |(x0, y0, x1, y1)| {
                (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
            }));
        }
    }
    let (x0, y0, x1, y1) = found?;
    Some((
        x0.saturating_sub(CROP_MARGIN),
        y0.saturating_sub(CROP_MARGIN),
        (x1 + CROP_MARGIN).min(width - 1),
        (y1 + CROP_MARGIN).min(height - 1),
    ))
}

/// An image zoomed to `area` of its `full` size, linked to the whole file.
pub(super) fn crop(
    src: &str,
    (full_width, full_height): (u32, u32),
    area: (u32, u32, u32, u32),
) -> String {
    let (x0, y0, x1, y1) = area;
    let width = f64::from(x1 - x0 + 1);
    let height = f64::from(y1 - y0 + 1);
    // Percent margins are relative to the box's width, on both axes.
    format!(
        "<a class=\"crop\" href=\"{src}\" style=\"aspect-ratio:{width}/{height}\"><img src=\"{src}\" alt=\"\" style=\"width:{:.3}%;height:auto;margin-left:-{:.3}%;margin-top:-{:.3}%\" data-full=\"{full_width}x{full_height}\"></a>",
        f64::from(full_width) / width * 100.0,
        f64::from(x0) / width * 100.0,
        f64::from(y0) / width * 100.0,
    )
}

fn image(src: &str, alt: &str) -> String {
    format!(
        "<a href=\"{src}\"><img src=\"{src}\" alt=\"{}\"></a>",
        escape(alt)
    )
}

/// Writes `index.html` into `out` and returns its path.
pub fn write(out: &Path) -> PathBuf {
    let (models, runs) = read_results(out);
    let title = out
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("comparison");
    let reference = std::fs::read_dir(out)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .find(|name| name.starts_with("reference."));
    let run = |model: &str, mode: &str| runs.get(&(model.to_owned(), mode.to_owned()));
    let file = |model: &str, mode: &str| {
        let relative = format!("{}/{mode}.png", slug(model));
        out.join(&relative).is_file().then_some(relative)
    };

    let mut html = String::new();
    let _ = write!(
        html,
        r#"<!doctype html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>画像モデルの比較 {title}</title>
<style>
:root {{ --bg:#f6f5f2; --card:#fff; --text:#1d1d1b; --muted:#6b6a66; --line:#dddbd5; --bad:#b3261e; }}
@media (prefers-color-scheme: dark) {{
  :root {{ --bg:#181817; --card:#232321; --text:#ecebe7; --muted:#a3a29c; --line:#3a3936; --bad:#f2b8b5; }}
}}
* {{ box-sizing:border-box; }}
body {{ margin:0; padding:24px 16px 64px; background:var(--bg); color:var(--text);
  font:15px/1.6 system-ui, "Segoe UI", "Hiragino Sans", "Yu Gothic UI", sans-serif; }}
main {{ max-width:1400px; margin:0 auto; }}
h1 {{ font-size:22px; margin:0 0 4px; }}
h2 {{ font-size:18px; margin:40px 0 12px; padding-top:12px; border-top:1px solid var(--line); }}
.muted, small {{ color:var(--muted); }}
.failed {{ color:var(--bad); font-weight:600; }}
header {{ display:flex; gap:20px; align-items:flex-start; flex-wrap:wrap; }}
header img {{ width:160px; border-radius:8px; border:1px solid var(--line); }}
.table {{ overflow-x:auto; }}
table {{ border-collapse:collapse; width:100%; background:var(--card); font-size:14px; }}
th, td {{ border:1px solid var(--line); padding:6px 10px; text-align:left; vertical-align:top; }}
th {{ white-space:nowrap; }}
.grid {{ display:grid; gap:12px; grid-template-columns:repeat(auto-fill, minmax(230px, 1fr)); }}
.card {{ background:var(--card); border:1px solid var(--line); border-radius:10px; padding:10px; min-width:0; }}
.card h3 {{ font-size:14px; margin:0 0 8px; overflow-wrap:anywhere; }}
.card img {{ display:block; width:100%; height:auto; border-radius:6px; }}
.wide {{ display:grid; gap:12px; }}
.pair {{ display:grid; grid-template-columns:1fr 1fr; gap:6px; }}
.pair figure {{ margin:0; }}
.pair figcaption {{ font-size:12px; color:var(--muted); text-align:center; }}
.crop {{ display:block; overflow:hidden; border-radius:6px; background:#fff; }}
.crop img {{ display:block; max-width:none; border-radius:0; }}
a {{ color:inherit; }}
</style>
</head>
<body>
<main>
<header>
"#
    );
    if let Some(reference) = &reference {
        let _ = write!(html, "{}", image(reference, "参考画像"));
    }
    let _ = write!(
        html,
        "<div><h1>画像モデルの比較</h1><p class=\"muted\">{} ・ {} モデル。画像を押すと元の大きさで開く。部分再生成は、直した範囲の前後を拡大して並べた。</p></div>\n</header>\n",
        escape(title),
        models.len()
    );

    // The numbers, one row per model.
    html.push_str("<h2>時間と料金</h2>\n<div class=\"table\"><table>\n<tr><th>モデル</th>");
    for (_, heading) in MODES {
        let _ = write!(html, "<th>{heading}</th>");
    }
    html.push_str("<th>計</th></tr>\n");
    for model in &models {
        let _ = write!(html, "<tr><th>{}</th>", escape(model));
        let mut total = 0.0;
        for (mode, _) in MODES {
            let found = run(model, mode);
            total += found.and_then(cost).unwrap_or(0.0);
            let _ = write!(html, "<td>{}</td>", summary(found));
        }
        let _ = writeln!(html, "<td>${total:.3}</td></tr>");
    }
    html.push_str("</table></div>\n");

    // Front views side by side.
    html.push_str("<h2>正面図</h2>\n<div class=\"grid\">\n");
    for model in &models {
        let body = file(model, "front")
            .map_or_else(|| summary(run(model, "front")), |src| image(&src, model));
        let _ = writeln!(
            html,
            "<div class=\"card\"><h3>{}</h3>{body}</div>",
            escape(model)
        );
    }
    html.push_str("</div>\n");

    // Turnarounds are wide: one per row.
    html.push_str("<h2>四面図</h2>\n<div class=\"wide\">\n");
    for model in &models {
        let body = file(model, "turnaround").map_or_else(
            || summary(run(model, "turnaround")),
            |src| image(&src, model),
        );
        let _ = writeln!(
            html,
            "<div class=\"card\"><h3>{}</h3>{body}</div>",
            escape(model)
        );
    }
    html.push_str("</div>\n");

    // Local revisions: the edited area before and after.
    html.push_str("<h2>部分再生成</h2>\n<div class=\"grid\">\n");
    for model in &models {
        let folder = out.join(slug(model));
        let body = match (
            file(model, "front"),
            file(model, "local"),
            crop_box(&folder.join("mask.png")),
        ) {
            (Some(before), Some(after), Some(area)) => {
                let size = super::super::FRONT_SIZE;
                format!(
                    "<div class=\"pair\"><figure>{}<figcaption>前</figcaption></figure><figure>{}<figcaption>後</figcaption></figure></div>",
                    crop(&before, size, area),
                    crop(&after, size, area),
                )
            }
            (_, Some(after), None) => image(&after, model),
            _ => summary(run(model, "local")),
        };
        let _ = writeln!(
            html,
            "<div class=\"card\"><h3>{}</h3>{body}</div>",
            escape(model)
        );
    }
    html.push_str("</div>\n");

    // Eye close-ups: the front view's eye band, then each close-up.
    if models
        .iter()
        .any(|model| run(model, "eyes").is_some() || run(model, "eyes-art").is_some())
    {
        html.push_str("<h2>目の拡大図</h2>\n<div class=\"wide\">\n");
        let size = super::super::FRONT_SIZE;
        let [left, top, right, bottom] = super::super::EYE_BAND;
        let band = (
            (f64::from(size.0) * left).round() as u32,
            (f64::from(size.1) * top).round() as u32,
            (f64::from(size.0) * right).round() as u32,
            (f64::from(size.1) * bottom).round() as u32,
        );
        for model in &models {
            let mut figures = Vec::new();
            if let Some(front) = file(model, "front") {
                figures.push(format!(
                    "<figure>{}<figcaption>正面図の同じあたり</figcaption></figure>",
                    crop(&front, size, band)
                ));
            }
            for (mode, heading) in [("eyes", "拡大図"), ("eyes-art", "拡大図（元の絵の目つき）")]
            {
                if let Some(src) = file(model, mode) {
                    figures.push(format!(
                        "<figure>{}<figcaption>{heading}</figcaption></figure>",
                        image(&src, heading)
                    ));
                }
            }
            let _ = writeln!(
                html,
                "<div class=\"card\"><h3>{}</h3><div class=\"wide\">{}</div></div>",
                escape(model),
                figures.join("")
            );
        }
        html.push_str("</div>\n");
    }
    html.push_str("</main>\n</body>\n</html>\n");

    let path = out.join("index.html");
    std::fs::write(&path, html).expect("report written");
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_lists_each_run_and_escapes_errors() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path();
        std::fs::create_dir_all(out.join("vendor_a")).unwrap();
        image::RgbImage::new(800, 1100)
            .save(out.join("vendor_a/front.png"))
            .unwrap();
        let mask = image::GrayImage::from_fn(800, 1100, |x, y| {
            image::Luma([if (100..200).contains(&x) && (300..400).contains(&y) {
                255
            } else {
                0
            }])
        });
        mask.save(out.join("vendor_a/mask.png")).unwrap();
        image::RgbImage::new(800, 1100)
            .save(out.join("vendor_a/local.png"))
            .unwrap();
        let lines = [
            r#"{"model":"vendor/a","mode":"front","ok":true,"seconds":12.3,"cost_usd":0.04,"size":[800,1100]}"#,
            r#"{"model":"vendor/a","mode":"local","ok":true,"seconds":20.0,"cost_usd":0.01,"size":[800,1100]}"#,
            r#"{"model":"vendor/b","mode":"front","ok":false,"seconds":1.0,"error":"<bad>"}"#,
        ];
        std::fs::write(out.join("results.jsonl"), lines.join("\n")).unwrap();

        let html = std::fs::read_to_string(write(out)).unwrap();
        assert!(html.contains("<th>vendor/a</th>"));
        assert!(html.contains("12 秒<br>$0.040<br>800×1100"));
        assert!(html.contains("<td>$0.050</td>"));
        assert!(html.contains("&lt;bad&gt;"));
        assert!(!html.contains("<bad>"));
        assert!(html.contains("src=\"vendor_a/front.png\""));
        // The crop is the mask's box plus the margin.
        assert_eq!(
            crop_box(&out.join("vendor_a/mask.png")),
            Some((40, 240, 259, 459))
        );
        assert!(html.contains("aspect-ratio:220/220"));
    }
}
