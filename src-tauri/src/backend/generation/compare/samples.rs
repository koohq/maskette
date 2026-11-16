//! Material samples: photos of real finished masks sent next to the
//! character, to see whether an image example brings the output closer to a
//! finished mask than the written look requirements alone.
//!
//! Only this comparison sends them; the app's prompts are unchanged. A run
//! goes through the same preparation as the app (`image_api::prepare`), then
//! adds the photos after the app's images, a line saying what they are after
//! the image roles, and swaps the "no finished-product image is attached"
//! style line for one that points at the photos.
//!
//! Each character gets its own output folder under one parent folder; the
//! `samples-report` step builds the parent's `index.html`, which puts every
//! character's runs and the judge's verdicts side by side.
//!
//! Use only photos you took yourself or have permission to use: they are
//! sent to the image model's provider along with the character.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::RgbImage;
use serde_json::{Value, json};

use super::super::hosted::HostedApi;
use super::super::image_api;
use super::super::prompt::{FRONT_NONE_STYLE_LINE, TURNAROUND_NONE_STYLE_LINE};
use super::super::{GenerationRequest, ImageBackend, Mode, image_backend};
use super::{Run, judge, report, slug};

/// The band of an 800x1100 front view that holds the eyes and cheeks.
const EYE_BAND: (u32, u32, u32, u32) = (150, 360, 649, 659);

const ONE_SAMPLE: &str = "{label} is a photo of a real, finished kigurumi head made by a professional \
workshop. It shows a different character: never copy its face, eye design, expression, hairstyle, hair \
colours, accessories or makeup. Use it only as the reference for physical materials and finish: the \
semi-gloss painted shell and its soft highlights, the glossy domed lens eyes over a finely printed iris, the \
painted lash line, and the sheen and clumping of the synthetic wig fibre.";

const SEVERAL_SAMPLES: &str = "{label} are photos of real, finished kigurumi heads made by professional \
workshops. They show different characters: never copy their faces, eye designs, expressions, hairstyles, \
hair colours, accessories or makeup. Use them only as the reference for physical materials and finish: the \
semi-gloss painted shell and its soft highlights, the glossy domed lens eyes over a finely printed iris, the \
painted lash line, and the sheen and clumping of the synthetic wig fibre.";

const FRONT_SAMPLE_STYLE_LINE: &str = "Finished-product photos of real kigurumi heads are attached as \
material samples. Match their physical product look together with these written requirements: white studio \
background, finished kigurumi head shell material, wig fiber realism, clean product framing, and \
product-photo lighting.";

const TURNAROUND_SAMPLE_STYLE_LINE: &str = "Finished-product photos of real kigurumi heads are attached as \
material samples. Build the four-view layout from the written requirements, and match the samples' physical \
product look: four evenly spaced views, white studio background, finished shell surface, and wig fiber \
realism.";

/// "Image 2", "Images 2 and 3", "Images 2, 3 and 4".
fn image_label(first: usize, count: usize) -> String {
    let numbers: Vec<String> = (first..first + count).map(|n| n.to_string()).collect();
    match numbers.as_slice() {
        [] => String::new(),
        [one] => format!("Image {one}"),
        [rest @ .., last] => format!("Images {} and {last}", rest.join(", ")),
    }
}

/// What the samples are, for `count` photos starting at image `first`.
pub fn describe(first: usize, count: usize) -> String {
    let template = if count == 1 {
        ONE_SAMPLE
    } else {
        SEVERAL_SAMPLES
    };
    template.replace("{label}", &image_label(first, count))
}

/// The app's prompt with the samples line after the image roles (the first
/// paragraph) and the style line that says no photo is attached swapped.
/// The second value says whether that style line was found.
pub fn with_samples(prompt: &str, line: &str) -> (String, bool) {
    let (roles, rest) = prompt.split_once("\n\n").unwrap_or((prompt, ""));
    let mut text = format!("{roles}\n{line}\n\n{rest}");
    let mut swapped = false;
    for (none, sample) in [
        (FRONT_NONE_STYLE_LINE, FRONT_SAMPLE_STYLE_LINE),
        (TURNAROUND_NONE_STYLE_LINE, TURNAROUND_SAMPLE_STYLE_LINE),
    ] {
        if text.contains(none) {
            text = text.replace(none, sample);
            swapped = true;
        }
    }
    (text.trim_end().to_owned(), swapped)
}

/// The output label of a stage run with `count` samples.
pub fn label(stage: &str, count: usize) -> String {
    if count == 0 {
        stage.to_owned()
    } else {
        format!("{stage}-samples{count}")
    }
}

/// Runs one job with `samples` added, keeps its output as
/// `<model>/<label>.png` and appends what happened to `results.jsonl`, like
/// `Run::job`.
pub async fn job(
    run: &Run<'_>,
    model: &str,
    label: &str,
    request: &GenerationRequest,
    samples: &[(String, RgbImage)],
) -> Option<PathBuf> {
    run.use_model(model);
    let state = &run.backend.state;
    let started = Instant::now();
    let mut prompt_text = None;
    let result: Result<(RgbImage, Option<f64>, bool), String> = async {
        let ImageBackend::Hosted(mut api) =
            image_backend(&state.settings.settings()).map_err(|failure| failure.message)?
        else {
            return Err("samples need a hosted image API".to_owned());
        };
        if let HostedApi::OpenRouter(openrouter) = &mut api {
            openrouter
                .load_parameters(&state.generation.client, &state.generation.models)
                .await;
        }
        let limit = api.reference_limit();
        let mut prepared = image_api::prepare(&state.data_dir, request, limit, api.prompt_limit())?;
        if prepared.images.len() + samples.len() > limit {
            return Err(format!(
                "{} images and {} samples are more than the model's {limit}",
                prepared.images.len(),
                samples.len()
            ));
        }
        let line = describe(prepared.images.len() + 1, samples.len());
        prepared
            .images
            .extend(samples.iter().map(|(_, image)| image.clone()));
        let (prompt, swapped) = with_samples(&prepared.prompt, &line);
        prepared.prompt = prompt;
        prompt_text = Some(prepared.prompt.clone());
        let trace_id = format!("maskette-compare-{}", slug(label));
        let api_request = api.api().build_request(
            &prepared.prompt,
            &prepared.images,
            &trace_id,
            request.generation_mode,
        );
        let answer = image_api::request_image(
            &state.generation.client,
            api.api(),
            &api_request,
            &trace_id,
            &state.generation.retry_delays,
        )
        .await
        .map_err(|failure| failure.message)?;
        let raw = image_api::decode_rgb(&answer.bytes)?;
        Ok((prepared.finish(&raw)?, answer.cost, swapped))
    }
    .await;
    let seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    let folder = run.out.join(slug(model));
    std::fs::create_dir_all(&folder).expect("output folder");
    if let Some(prompt) = &prompt_text {
        std::fs::write(folder.join(format!("{label}-prompt.md")), prompt).expect("prompt written");
    }
    let (output, mut line) = match &result {
        Ok((image, cost, swapped)) => {
            let path = folder.join(format!("{label}.png"));
            image.save(&path).expect("output written");
            (
                Some(path),
                json!({
                    "ok": true,
                    "size": [image.width(), image.height()],
                    "cost_usd": cost,
                    "style_line_swapped": swapped,
                    "error": null,
                }),
            )
        }
        Err(error) => (None, json!({"ok": false, "error": error})),
    };
    let names: Vec<&str> = samples.iter().map(|(name, _)| name.as_str()).collect();
    line["model"] = json!(model);
    line["mode"] = json!(label);
    line["seconds"] = json!(seconds);
    line["samples"] = json!(names);
    println!("{line}");
    let mut results = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run.out.join("results.jsonl"))
        .expect("results file");
    writeln!(results, "{line}").expect("result written");
    output
}

/// The sample photos, copied next to the results as `sample-<n>.<ext>` so
/// the page can show them.
pub fn load_samples(paths: &[PathBuf], out: &Path) -> Vec<(String, RgbImage)> {
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("png")
                .to_ascii_lowercase();
            let name = format!("sample-{}.{extension}", index + 1);
            std::fs::copy(path, out.join(&name)).expect("sample copied");
            let image = image_api::load_rgb(path).expect("sample readable");
            (name, image)
        })
        .collect()
}

/// Runs `stage` (front or turnaround) once without samples and once with
/// each count of samples.
pub async fn run_stage(
    run: &Run<'_>,
    models: &[String],
    stage: &str,
    reference: &Path,
    reference_name: &str,
    samples: &[(String, RgbImage)],
    counts: &[usize],
) {
    for model in models {
        for &count in counts {
            let label = label(stage, count);
            let request = match stage {
                "front" => {
                    let mut request = GenerationRequest::new("compare", "ja", Mode::FrontDesign);
                    request.reference_keys =
                        vec![run.reference("front", reference, reference_name)];
                    request
                }
                "turnaround" => {
                    // Each turnaround follows the front view of the same count.
                    let front = run
                        .out
                        .join(slug(model))
                        .join(format!("{}.png", self::label("front", count)));
                    if !front.is_file() {
                        println!("no {} for {model}; skipping", front.display());
                        continue;
                    }
                    let mut request = GenerationRequest::new("compare", "ja", Mode::Turnaround);
                    request.reference_keys = vec![
                        run.reference("front", &front, &format!("{}-{label}.png", slug(model))),
                        run.reference("supplemental", reference, reference_name),
                    ];
                    request
                }
                other => {
                    panic!("MASKETTE_COMPARE_SAMPLE_STAGE is front or turnaround, not {other:?}")
                }
            };
            if count == 0 {
                // Exactly the app's path.
                run.job(model, &label, &request).await;
            } else {
                job(run, model, &label, &request, &samples[..count]).await;
            }
        }
    }
}

// --- Page ------------------------------------------------------------------

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// "A：今のまま", "B：見本 1 枚", ... for a run label.
fn caption(label: &str) -> String {
    let count = label
        .rsplit_once("-samples")
        .and_then(|(_, count)| count.parse::<usize>().ok())
        .unwrap_or(0);
    let letter = char::from(b'A' + u8::try_from(count.min(25)).unwrap_or(0));
    if count == 0 {
        format!("{letter}：今のまま")
    } else {
        format!("{letter}：見本 {count} 枚")
    }
}

/// The last run of each mode in a character folder, in the order first run.
fn runs(folder: &Path) -> Vec<(String, Value)> {
    let text = std::fs::read_to_string(folder.join("results.jsonl")).unwrap_or_default();
    let mut runs: Vec<(String, Value)> = Vec::new();
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
        let key = format!("{}/{mode}", slug(model));
        match runs.iter_mut().find(|(known, _)| *known == key) {
            Some((_, slot)) => *slot = record,
            None => runs.push((key, record)),
        }
    }
    runs
}

fn figure(src: &str, title: &str, note: &str) -> String {
    format!(
        "<figure><a href=\"{src}\"><img src=\"{src}\" alt=\"{}\" loading=\"lazy\"></a><figcaption><b>{}</b> <small>{}</small></figcaption></figure>",
        escape(title),
        escape(title),
        escape(note)
    )
}

fn run_note(run: &Value) -> String {
    if run.get("ok").and_then(Value::as_bool) != Some(true) {
        return format!(
            "失敗 {}",
            run.get("error").and_then(Value::as_str).unwrap_or("")
        );
    }
    let mut parts = Vec::new();
    if let Some(seconds) = run.get("seconds").and_then(Value::as_f64) {
        parts.push(format!("{seconds:.0} 秒"));
    }
    if let Some(cost) = run.get("cost_usd").and_then(Value::as_f64) {
        parts.push(format!("${cost:.3}"));
    }
    parts.join(" ・ ")
}

/// The character folders under `parent`: those with a `results.jsonl`.
fn characters(parent: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("results.jsonl").is_file())
        .collect();
    folders.sort();
    folders
}

fn file_starting(folder: &Path, prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(prefix))
        .collect();
    names.sort();
    names
}

/// Writes `index.html` into `parent` and returns its path.
pub fn write_page(parent: &Path) -> PathBuf {
    let title = parent
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("samples");
    let folders = characters(parent);
    let mut html = String::new();
    let _ = write!(
        html,
        r#"<!doctype html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>見本写真の比較 {title}</title>
<style>
:root {{ --bg:#f6f5f2; --card:#fff; --text:#1d1d1b; --muted:#6b6a66; --line:#dddbd5; --good:#1e6b3a; --bad:#b3261e; }}
@media (prefers-color-scheme: dark) {{
  :root {{ --bg:#181817; --card:#232321; --text:#ecebe7; --muted:#a3a29c; --line:#3a3936; --good:#9fd8ad; --bad:#f2b8b5; }}
}}
* {{ box-sizing:border-box; }}
body {{ margin:0; padding:24px 16px 64px; background:var(--bg); color:var(--text);
  font:15px/1.6 system-ui, "Segoe UI", "Hiragino Sans", "Yu Gothic UI", sans-serif; }}
main {{ max-width:1500px; margin:0 auto; }}
h1 {{ font-size:22px; margin:0 0 4px; }}
h2 {{ font-size:18px; margin:40px 0 12px; padding-top:12px; border-top:1px solid var(--line); }}
h3 {{ font-size:15px; margin:16px 0 8px; }}
h4 {{ font-size:14px; margin:12px 0 6px; }}
.muted, small {{ color:var(--muted); }}
.row {{ display:grid; gap:12px; grid-template-columns:repeat(auto-fill, minmax(220px, 1fr)); }}
.wide {{ display:grid; gap:12px; }}
.eyes {{ display:grid; gap:12px; max-width:1000px; }}
figure {{ margin:0; background:var(--card); border:1px solid var(--line); border-radius:10px; padding:8px; min-width:0; }}
figure img {{ display:block; width:100%; height:auto; border-radius:6px; }}
figcaption {{ font-size:13px; margin-top:4px; }}
.table {{ overflow-x:auto; }}
table {{ border-collapse:collapse; background:var(--card); font-size:13px; }}
th, td {{ border:1px solid var(--line); padding:4px 8px; text-align:center; white-space:nowrap; }}
th:first-child, td:first-child {{ text-align:left; }}
.pos {{ color:var(--good); font-weight:600; }}
.neg {{ color:var(--bad); font-weight:600; }}
.split {{ text-decoration:underline dotted; }}
.crop {{ display:block; overflow:hidden; border-radius:6px; background:#fff; }}
.crop img {{ display:block; max-width:none; border-radius:0; }}
a {{ color:inherit; }}
</style>
</head>
<body>
<main>
<h1>見本写真の比較</h1>
<p class="muted">{title}。A は今のプロンプトのまま、B・C は本物の面の写真を「材質・仕上げの見本」として 1 枚・2 枚足したもの。画像を押すと元の大きさで開く。</p>
"#,
        title = escape(title)
    );

    // The sample photos, from the first folder that has them.
    if let Some(folder) = folders
        .iter()
        .find(|folder| !file_starting(folder, "sample-").is_empty())
    {
        let name = folder.file_name().unwrap_or_default().to_string_lossy();
        html.push_str("<h2>見本の写真</h2>\n<div class=\"row\">\n");
        for sample in file_starting(folder, "sample-") {
            let src = format!("{name}/{sample}");
            let _ = writeln!(html, "{}", figure(&src, &sample, ""));
        }
        html.push_str("</div>\n");
    }

    // The judge's verdicts over every character.
    let mut all = Vec::new();
    for folder in &folders {
        all.extend(judge::read(folder));
    }
    if !all.is_empty() {
        html.push_str("<h2>採点（全キャラの合計）</h2>\n<p class=\"muted\">2 枚を並べてどちらが良いかを、左右を入れ替えた 2 順で聞いた平均。+1 は 2 順とも後ろの組が良い、0 は同じか割れた。</p>\n");
        html.push_str(&judge::tables(&all));
    }

    for folder in &folders {
        let name = folder.file_name().unwrap_or_default().to_string_lossy();
        let _ = writeln!(html, "<h2>{}</h2>", escape(&name));
        let runs = runs(folder);
        for (stage, heading, class) in
            [("front", "正面図", "row"), ("turnaround", "四面図", "wide")]
        {
            let stage_runs: Vec<&(String, Value)> = runs
                .iter()
                .filter(|(key, _)| {
                    let mode = key.rsplit_once('/').map_or(key.as_str(), |(_, mode)| mode);
                    mode == stage || mode.starts_with(&format!("{stage}-samples"))
                })
                .collect();
            if stage_runs.is_empty() {
                continue;
            }
            let _ = writeln!(html, "<h3>{heading}</h3>\n<div class=\"{class}\">");
            if stage == "front"
                && let Some(reference) = file_starting(folder, "reference.").first()
            {
                let src = format!("{name}/{reference}");
                let _ = writeln!(html, "{}", figure(&src, "元の絵", ""));
            }
            for (key, run) in stage_runs {
                let mode = key.rsplit_once('/').map_or(key.as_str(), |(_, mode)| mode);
                let src = format!("{name}/{key}.png");
                let body = if folder.join(format!("{key}.png")).is_file() {
                    figure(&src, &caption(mode), &run_note(run))
                } else {
                    format!(
                        "<figure><figcaption><b>{}</b> {}</figcaption></figure>",
                        escape(&caption(mode)),
                        escape(&run_note(run))
                    )
                };
                let _ = writeln!(html, "{body}");
            }
            html.push_str("</div>\n");
            if stage == "front" {
                // Small borrowed details (marks under the eyes, iris
                // patterns) only show when the eyes are enlarged.
                html.push_str("<h3>目元の拡大</h3>\n<div class=\"eyes\">\n");
                for (key, _) in runs.iter().filter(|(key, _)| {
                    let mode = key.rsplit_once('/').map_or(key.as_str(), |(_, mode)| mode);
                    mode == "front" || mode.starts_with("front-samples")
                }) {
                    if !folder.join(format!("{key}.png")).is_file() {
                        continue;
                    }
                    let mode = key.rsplit_once('/').map_or(key.as_str(), |(_, mode)| mode);
                    let _ = writeln!(
                        html,
                        "<figure>{}<figcaption><b>{}</b></figcaption></figure>",
                        report::crop(
                            &format!("{name}/{key}.png"),
                            super::super::FRONT_SIZE,
                            EYE_BAND
                        ),
                        escape(&caption(mode))
                    );
                }
                html.push_str("</div>\n");
            }
        }
        let verdicts = judge::read(folder);
        if !verdicts.is_empty() {
            html.push_str("<h3>採点</h3>\n");
            html.push_str(&judge::tables(&verdicts));
        }
    }

    // What the runs cost.
    let mut total = 0.0;
    let mut by_kind: BTreeMap<&str, f64> = BTreeMap::new();
    for folder in &folders {
        for (_, run) in runs(folder) {
            let cost = run.get("cost_usd").and_then(Value::as_f64).unwrap_or(0.0);
            *by_kind.entry("画像").or_default() += cost;
            total += cost;
        }
        for verdict in judge::read(folder) {
            let cost = verdict.cost.unwrap_or(0.0);
            *by_kind.entry("採点").or_default() += cost;
            total += cost;
        }
    }
    let parts: Vec<String> = by_kind
        .iter()
        .map(|(kind, cost)| format!("{kind} ${cost:.3}"))
        .collect();
    let _ = writeln!(
        html,
        "<h2>費用</h2>\n<p>計 ${total:.3}（{}）</p>\n</main>\n</body>\n</html>",
        parts.join("、")
    );

    let path = parent.join("index.html");
    std::fs::write(&path, html).expect("page written");
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_samples_line_follows_the_roles_and_replaces_the_no_photo_line() {
        let prompt = format!(
            "Image 1 is the character reference.\n\nStage text.\n{FRONT_NONE_STYLE_LINE}\n\nUser notes."
        );
        let (text, swapped) = with_samples(&prompt, &describe(2, 2));
        assert!(swapped);
        assert!(text.starts_with(
            "Image 1 is the character reference.\nImages 2 and 3 are photos of real, finished kigurumi heads"
        ));
        assert!(!text.contains("No finished-product reference image is attached."));
        assert!(text.contains(FRONT_SAMPLE_STYLE_LINE));
        assert!(text.ends_with("User notes."));
        assert!(describe(2, 1).starts_with("Image 2 is a photo of a real"));
        assert_eq!(image_label(2, 3), "Images 2, 3 and 4");
    }

    #[test]
    fn labels_and_captions_count_the_samples() {
        assert_eq!(label("front", 0), "front");
        assert_eq!(label("turnaround", 2), "turnaround-samples2");
        assert_eq!(caption("front"), "A：今のまま");
        assert_eq!(caption("front-samples1"), "B：見本 1 枚");
        assert_eq!(caption("turnaround-samples2"), "C：見本 2 枚");
    }
}
