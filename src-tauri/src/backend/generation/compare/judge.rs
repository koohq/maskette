//! A second opinion on two front views from a vision model, item by item:
//! which one looks more like a real finished mask (`realism`), and which
//! one keeps the character art better (`fidelity`).
//!
//! Scores of single images bunch in the middle and do not separate runs, so
//! the judge always sees two images side by side, once in each order; the
//! page averages both orders, because some items flip with the order. The
//! fidelity judge also sees the sample photos and says, for each image,
//! whether it took a design feature from them that the art does not have
//! (the run without samples answers too, as a control).
//!
//! ```text
//! MASKETTE_COMPARE_STEP=judge
//! MASKETTE_COMPARE_OUT=<a character folder of the samples step>
//! MASKETTE_COMPARE_MODELS=<the image model whose runs are compared>
//! MASKETTE_COMPARE_PAIRS=front:front-samples1,front:front-samples2
//! MASKETTE_COMPARE_JUDGE=google/gemini-3.8-flash   (optional)
//! MASKETTE_COMPARE_JUDGE_KINDS=realism,fidelity    (optional)
//! ```

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::super::image_api;
use super::slug;

pub const DEFAULT_MODEL: &str = "google/gemini-3.8-flash";
const URL: &str = "https://openrouter.ai/api/v1/chat/completions";
/// Stops the step when the judge has cost more than this (USD).
const MAX_COST: f64 = 0.3;

/// The realism items, with their headings on the page.
const REALISM_ITEMS: [(&str, &str); 8] = [
    ("lens_eyes", "レンズ目"),
    ("mouth", "口"),
    ("nose_face", "鼻と輪郭"),
    ("hair_strands", "毛束"),
    ("shell_finish", "つや"),
    ("skin_tone", "肌の色"),
    ("presentation", "見せ方"),
    ("overall", "全体"),
];

const FIDELITY_ITEMS: [(&str, &str); 4] = [
    ("eyes_face", "目と顔"),
    ("hair", "髪形と色"),
    ("accessories", "付属物"),
    ("overall", "全体"),
];

const REALISM_PROMPT: &str = "You are comparing two AI-generated product photos of animegao kigurumi heads \
(a hard painted anime-face head shell plus a synthetic wig), Image A and Image B. Judge ONLY how much each \
looks like a real finished mask that a professional kigurumi workshop would make and photograph, using the \
rubric below. Do NOT judge faithfulness to any character art, and do not penalize a missing stand or neck \
mount: the head is shown floating on purpose.

lens_eyes:
  best = both eyes look like glossy domed lenses with window-shaped or band reflections over a finely printed \
iris, and a thick black painted upper lash line.
  worse = flat printed or painted eyes with only drawn highlights, or the lens look is unclear.
  worst = realistic human eyes (wet eyeballs, veins, real hair lashes).
mouth:
  best = small relative to the face. Closed: a painted line or very shallow carving. Open: recessed into the \
shell with the inside painted red or pink (a small tongue or fang is fine).
  worse = somewhat too large, or an open mouth that is only flat paint.
  worst = a large mouth, human-like lips or rows of teeth.
nose_face:
  best = the nose is only a tiny bump or a shadow; no lips, cheekbones or eyelid creases; a small round chin \
and flat cheeks.
  worse = nose or cheeks carved a bit too human.
  worst = a human nose, lips, cheekbones or wrinkles.
hair_strands:
  best = bangs and sidelocks grouped into anime-style clumps of a few strands framing the face; \
synthetic-fiber sheen; a smooth crown; the bangs stay off the eyes. The bang length itself and whether the \
eyebrows show are NOT judged.
  worse = weak clumping but still reads as a wig, or loose strands fall over the eyes.
  worst = realistic individual human hair, or a single solid sculpted or plastic hair mass.
shell_finish:
  best = semi-gloss: soft highlight bands on the forehead, nose tip and cheeks; smooth paint.
  worse = almost fully matte with no highlight bands.
  worst = wet-looking strong gloss, or human skin texture (pores, fine skin detail).
skin_tone:
  best = near-white porcelain with a light pink or peach tint.
  worse = warm beige or tan, or somewhat too saturated or yellow.
  worst = greyish, unnaturally saturated, or a photographic human skin colour.
presentation:
  best = head only; plain light background; soft frontal light; facing front.
  worse = a little of the neck base shows, or the background or lighting is slightly off.
  worst = a neck tube opening, body or clothing is visible; or harsh side light or a dark background.
overall:
  best = could pass as a workshop's photo of its finished product.
  worst = does not read as a kigurumi mask at all.

For every item answer which image fits the best description better: \"A\", \"B\", or \"same\" when you \
cannot see a clear difference. Each reason: one short Japanese sentence (about 40 characters at most) \
naming what you saw.";

const FIDELITY_PROMPT: &str = "You are comparing two AI-generated front views of a kigurumi head, Image A \
and Image B, both made from the character art shown first. Judge ONLY how faithfully each keeps the \
character's own design from the art. Do NOT judge how realistic or photographic it looks, and do not \
penalize the translation into a physical mask itself (a painted shell, lens eyes, wig fibre, a head with no \
body or clothing).

eyes_face: the eye shape, size, iris colours and pattern, highlights, lash style, brows, expression, mouth \
and face shape of the art.
hair: the hairstyle, bang shape and length, parting, hair colours, and distinctive locks, braids or ties of \
the art.
accessories: the head accessories, ears and other features of the art, with their shapes, colours and \
placement; nothing added that the art does not have.
overall: which one a maker could build this character from more faithfully.

For every item answer which image keeps the art better: \"A\", \"B\", or \"same\" when you cannot see a \
clear difference.

The photos shown last are real masks of OTHER characters. They were given to the generator only as \
examples of materials and finish. For each of Image A and Image B, say whether it took a design feature \
from those photos that the character art does not have (face shape, eye design, makeup, expression, \
hairstyle, hair colour, accessories): \"none\", \"slight\" or \"clear\", and name it. Each reason: one short \
Japanese sentence (about 40 characters at most).";

/// What a judge looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Realism,
    Fidelity,
}

impl Kind {
    pub const ALL: [Self; 2] = [Self::Realism, Self::Fidelity];

    pub fn name(self) -> &'static str {
        match self {
            Self::Realism => "realism",
            Self::Fidelity => "fidelity",
        }
    }

    pub fn heading(self) -> &'static str {
        match self {
            Self::Realism => "実物らしさ",
            Self::Fidelity => "元の絵への忠実さ",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    pub fn items(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Realism => &REALISM_ITEMS,
            Self::Fidelity => &FIDELITY_ITEMS,
        }
    }

    fn schema(self) -> Value {
        let item = json!({
            "type": "object",
            "properties": {
                "better": {"type": "string", "enum": ["A", "B", "same"]},
                "reason": {"type": "string"},
            },
            "required": ["better", "reason"],
            "additionalProperties": false,
        });
        let mut properties = serde_json::Map::new();
        for (name, _) in self.items() {
            properties.insert((*name).to_owned(), item.clone());
        }
        let mut required: Vec<&str> = self.items().iter().map(|(name, _)| *name).collect();
        if self == Self::Fidelity {
            let borrowed = json!({
                "type": "object",
                "properties": {
                    "level": {"type": "string", "enum": ["none", "slight", "clear"]},
                    "what": {"type": "string"},
                },
                "required": ["level", "what"],
                "additionalProperties": false,
            });
            properties.insert("borrowed_A".to_owned(), borrowed.clone());
            properties.insert("borrowed_B".to_owned(), borrowed);
            required.extend(["borrowed_A", "borrowed_B"]);
        }
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    }
}

/// "a:b,c:d" as pairs of run labels.
pub fn parse_pairs(text: &str) -> Vec<(String, String)> {
    text.split(',')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let (first, second) = part.split_once(':').expect("first:second");
            (first.trim().to_owned(), second.trim().to_owned())
        })
        .collect()
}

/// "realism,fidelity" as kinds; every kind when unset.
pub fn parse_kinds(text: Option<&str>) -> Vec<Kind> {
    text.map_or_else(
        || Kind::ALL.to_vec(),
        |text| {
            text.split(',')
                .filter(|name| !name.trim().is_empty())
                .map(|name| Kind::from_name(name.trim()).expect("realism or fidelity"))
                .collect()
        },
    )
}

/// The judge's JSON answer, from a reply that may be fenced.
fn parse_answer(text: &str) -> Result<Value, String> {
    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .map_or(text, |inner| inner.trim_end_matches('`'));
    serde_json::from_str(text.trim()).map_err(|error| format!("unreadable answer: {error}"))
}

/// +1 when the second run of the pair was judged better, -1 when the first
/// was, 0 for "same"; `second_shown_first` says which one was Image A.
fn delta(better: &str, second_shown_first: bool) -> i8 {
    let second_letter = if second_shown_first { "A" } else { "B" };
    match better {
        "same" => 0,
        letter if letter == second_letter => 1,
        _ => -1,
    }
}

fn data_uri(path: &Path) -> Result<Value, String> {
    let uri = image_api::to_data_uri(&image_api::load_rgb(path)?, 1536);
    Ok(json!({"type": "image_url", "image_url": {"url": uri}}))
}

/// Files in `folder` whose names start with `prefix`, sorted.
fn files_starting(folder: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(prefix))
        })
        .collect();
    files.sort();
    files
}

async fn ask(
    client: &reqwest::Client,
    key: &str,
    judge_model: &str,
    kind: Kind,
    out: &Path,
    image_a: &Path,
    image_b: &Path,
) -> Result<(Value, Option<f64>), String> {
    let mut content = Vec::new();
    match kind {
        Kind::Realism => content.push(json!({"type": "text", "text": REALISM_PROMPT})),
        Kind::Fidelity => {
            let art = files_starting(out, "reference.")
                .into_iter()
                .next()
                .ok_or("no character art (reference.*) in the folder")?;
            content.push(json!({"type": "text", "text": FIDELITY_PROMPT}));
            content.push(json!({"type": "text", "text": "Character art:"}));
            content.push(data_uri(&art)?);
        }
    }
    content.push(json!({"type": "text", "text": "Image A:"}));
    content.push(data_uri(image_a)?);
    content.push(json!({"type": "text", "text": "Image B:"}));
    content.push(data_uri(image_b)?);
    if kind == Kind::Fidelity {
        content.push(json!({"type": "text", "text": "Photos of real masks of other characters (material examples only):"}));
        for sample in files_starting(out, "sample-") {
            content.push(data_uri(&sample)?);
        }
    }
    let body = json!({
        "model": judge_model,
        "messages": [{"role": "user", "content": content}],
        "response_format": {
            "type": "json_schema",
            "json_schema": {"name": format!("mask_{}", kind.name()), "strict": true, "schema": kind.schema()},
        },
        "reasoning": {"effort": "low"},
        "usage": {"include": true},
    });
    let response = client
        .post(URL)
        .bearer_auth(key)
        .timeout(Duration::from_secs(180))
        .json(&body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let reply: Value = response.json().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        let detail: String = reply.to_string().chars().take(400).collect();
        return Err(format!("HTTP {status}: {detail}"));
    }
    let content = reply
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or("no answer text")?;
    let cost = reply.pointer("/usage/cost").and_then(Value::as_f64);
    Ok((parse_answer(content)?, cost))
}

/// The judge's answer as one line of `judge.jsonl`.
fn record(
    kind: Kind,
    answer: &Value,
    first: &str,
    second: &str,
    second_shown_first: bool,
) -> Value {
    let mut deltas = serde_json::Map::new();
    let mut reasons = serde_json::Map::new();
    for (name, _) in kind.items() {
        let better = answer
            .pointer(&format!("/{name}/better"))
            .and_then(Value::as_str)
            .unwrap_or("same");
        deltas.insert((*name).to_owned(), json!(delta(better, second_shown_first)));
        reasons.insert(
            (*name).to_owned(),
            answer
                .pointer(&format!("/{name}/reason"))
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
    let mut line = json!({"delta": deltas, "reasons": reasons});
    if kind == Kind::Fidelity {
        let (label_a, label_b) = if second_shown_first {
            (second, first)
        } else {
            (first, second)
        };
        let mut borrowed = serde_json::Map::new();
        for (label, letter) in [(label_a, "A"), (label_b, "B")] {
            borrowed.insert(
                label.to_owned(),
                answer
                    .get(format!("borrowed_{letter}"))
                    .cloned()
                    .unwrap_or(Value::Null),
            );
        }
        line["borrowed"] = Value::Object(borrowed);
    }
    line
}

/// Compares each pair of runs of each model, once in each order, for each
/// kind, and appends the verdicts to `judge.jsonl` in `out`.
pub async fn run(
    client: &reqwest::Client,
    key: &str,
    judge_model: &str,
    kinds: &[Kind],
    out: &Path,
    models: &[String],
    pairs: &[(String, String)],
) {
    let mut spent = 0.0;
    for model in models {
        let folder = out.join(slug(model));
        for (first, second) in pairs {
            let (first_path, second_path) = (
                folder.join(format!("{first}.png")),
                folder.join(format!("{second}.png")),
            );
            if !first_path.is_file() || !second_path.is_file() {
                println!("{model}: {first} or {second} is missing; skipping");
                continue;
            }
            for &kind in kinds {
                for second_shown_first in [false, true] {
                    let (image_a, image_b) = if second_shown_first {
                        (&second_path, &first_path)
                    } else {
                        (&first_path, &second_path)
                    };
                    let started = Instant::now();
                    let answer = ask(client, key, judge_model, kind, out, image_a, image_b).await;
                    let seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
                    let mut line = match &answer {
                        Ok((answer, _)) => record(kind, answer, first, second, second_shown_first),
                        Err(error) => json!({"error": error}),
                    };
                    let cost = answer.as_ref().ok().and_then(|(_, cost)| *cost);
                    spent += cost.unwrap_or(0.0);
                    line["kind"] = json!(kind.name());
                    line["model"] = json!(model);
                    line["first"] = json!(first);
                    line["second"] = json!(second);
                    line["image_a"] = json!(if second_shown_first { second } else { first });
                    line["judge"] = json!(judge_model);
                    line["seconds"] = json!(seconds);
                    line["cost"] = json!(cost);
                    if line.get("error").is_none() {
                        line["error"] = Value::Null;
                    }
                    println!("{line}");
                    let mut file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(out.join("judge.jsonl"))
                        .expect("judge file");
                    writeln!(file, "{line}").expect("verdict written");
                    assert!(
                        spent <= MAX_COST,
                        "the judge has cost ${spent:.3}, more than ${MAX_COST}; stopping"
                    );
                }
            }
        }
    }
    println!("judge cost: ${spent:.4}");
}

/// One verdict read back from `judge.jsonl`.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub kind: Kind,
    pub character: String,
    pub model: String,
    pub first: String,
    pub second: String,
    pub second_shown_first: bool,
    pub deltas: BTreeMap<String, i8>,
    /// Run label -> "none", "slight" or "clear" (fidelity only).
    pub borrowed: BTreeMap<String, String>,
    pub cost: Option<f64>,
}

/// The verdicts in a character folder that have an answer.
pub fn read(folder: &Path) -> Vec<Verdict> {
    let character = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = std::fs::read_to_string(folder.join("judge.jsonl")).unwrap_or_default();
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line.get("error").is_none_or(Value::is_null))
        .filter_map(|line| {
            let text = |key: &str| line.get(key).and_then(Value::as_str).map(str::to_owned);
            let (model, first, second, image_a) = (
                text("model")?,
                text("first")?,
                text("second")?,
                text("image_a")?,
            );
            let kind = Kind::from_name(&text("kind").unwrap_or_else(|| "realism".to_owned()))?;
            let deltas = line
                .get("delta")?
                .as_object()?
                .iter()
                .filter_map(|(name, value)| {
                    Some((name.clone(), i8::try_from(value.as_i64()?).ok()?))
                })
                .collect();
            let borrowed = line
                .get("borrowed")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .filter_map(|(label, value)| {
                    Some((label.clone(), value.get("level")?.as_str()?.to_owned()))
                })
                .collect();
            Some(Verdict {
                kind,
                character: character.clone(),
                model,
                second_shown_first: image_a == second,
                first,
                second,
                deltas,
                borrowed,
                cost: line.get("cost").and_then(Value::as_f64),
            })
        })
        .collect()
}

fn letter(label: &str) -> char {
    let count = label
        .rsplit_once("-samples")
        .and_then(|(_, count)| count.parse::<u8>().ok())
        .unwrap_or(0);
    char::from(b'A' + count.min(25))
}

/// The verdicts of `kind` per pair as a table: the mean over both orders
/// (and every character), positive when the second run was judged better.
pub fn table(verdicts: &[Verdict], kind: Kind) -> String {
    let verdicts: Vec<&Verdict> = verdicts
        .iter()
        .filter(|verdict| verdict.kind == kind)
        .collect();
    if verdicts.is_empty() {
        return String::new();
    }
    let items = kind.items();
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut sums: BTreeMap<(String, String), BTreeMap<&str, Vec<i8>>> = BTreeMap::new();
    // (first, second, character, model) -> item -> the answers with the
    // first and with the second run shown first.
    type Runs = (String, String, String, String);
    let mut orders: BTreeMap<Runs, BTreeMap<&str, [Option<i8>; 2]>> = BTreeMap::new();
    for verdict in &verdicts {
        let pair = (verdict.first.clone(), verdict.second.clone());
        if !pairs.contains(&pair) {
            pairs.push(pair.clone());
        }
        let key = (
            pair.0.clone(),
            pair.1.clone(),
            verdict.character.clone(),
            verdict.model.clone(),
        );
        for (name, _) in items {
            let Some(&delta) = verdict.deltas.get(*name) else {
                continue;
            };
            sums.entry(pair.clone())
                .or_default()
                .entry(name)
                .or_default()
                .push(delta);
            orders
                .entry(key.clone())
                .or_default()
                .entry(name)
                .or_default()[usize::from(verdict.second_shown_first)] = Some(delta);
        }
    }

    let mut html = format!(
        "<h4>{}</h4>\n<div class=\"table\"><table>\n<tr><th>比べた組（+ は後ろが良い）</th><th>回数</th>",
        kind.heading()
    );
    for (_, heading) in items {
        let _ = write!(html, "<th>{heading}</th>");
    }
    html.push_str("<th>左右で割れた</th></tr>\n");
    for pair in &pairs {
        let by_item = &sums[pair];
        let count = by_item.values().map(Vec::len).max().unwrap_or(0);
        let _ = write!(
            html,
            "<tr><td>{} → {}</td><td>{count}</td>",
            letter(&pair.0),
            letter(&pair.1)
        );
        for (name, _) in items {
            let values = by_item.get(name).map(Vec::as_slice).unwrap_or_default();
            if values.is_empty() {
                html.push_str("<td>—</td>");
                continue;
            }
            let mean =
                values.iter().map(|value| f64::from(*value)).sum::<f64>() / values.len() as f64;
            let class = if mean > 0.0 {
                "pos"
            } else if mean < 0.0 {
                "neg"
            } else {
                ""
            };
            let _ = write!(html, "<td class=\"{class}\">{mean:+.2}</td>");
        }
        // Items whose two orders gave different answers.
        let (mut split, mut both) = (0, 0);
        for ((first, second, _, _), answers) in &orders {
            if (first, second) != (&pair.0, &pair.1) {
                continue;
            }
            for answer in answers.values() {
                if let [Some(a), Some(b)] = answer {
                    both += 1;
                    if a != b {
                        split += 1;
                    }
                }
            }
        }
        let _ = writeln!(html, "<td>{split}/{both}</td></tr>");
    }
    html.push_str("</table></div>\n");

    // How often each run was said to take something from the samples.
    let mut borrowed: BTreeMap<String, [usize; 3]> = BTreeMap::new();
    for verdict in &verdicts {
        for (label, level) in &verdict.borrowed {
            let slot = borrowed.entry(label.clone()).or_default();
            match level.as_str() {
                "clear" => slot[2] += 1,
                "slight" => slot[1] += 1,
                _ => slot[0] += 1,
            }
        }
    }
    if !borrowed.is_empty() {
        html.push_str("<p class=\"muted\">見本の面から持ち込んだ（なし・少し・はっきり）：");
        let parts: Vec<String> = borrowed
            .iter()
            .map(|(label, [none, slight, clear])| {
                format!("{} {none}・{slight}・{clear}", letter(label))
            })
            .collect();
        html.push_str(&parts.join(" ／ "));
        html.push_str("</p>\n");
    }
    html
}

/// Every kind's table.
pub fn tables(verdicts: &[Verdict]) -> String {
    Kind::ALL
        .into_iter()
        .map(|kind| table(verdicts, kind))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_follow_the_second_run_whichever_side_it_was_on() {
        assert_eq!(delta("B", false), 1);
        assert_eq!(delta("A", false), -1);
        assert_eq!(delta("A", true), 1);
        assert_eq!(delta("B", true), -1);
        assert_eq!(delta("same", true), 0);
    }

    #[test]
    fn answers_pairs_and_kinds_parse() {
        let answer = parse_answer("```json\n{\"overall\": {\"better\": \"A\"}}\n```").unwrap();
        assert_eq!(answer["overall"]["better"], "A");
        assert_eq!(
            parse_pairs("front:front-samples1, front:front-samples2"),
            vec![
                ("front".to_owned(), "front-samples1".to_owned()),
                ("front".to_owned(), "front-samples2".to_owned()),
            ]
        );
        assert_eq!(parse_kinds(None), Kind::ALL.to_vec());
        assert_eq!(parse_kinds(Some("fidelity")), vec![Kind::Fidelity]);
        assert_eq!(letter("front-samples2"), 'C');
    }

    #[test]
    fn borrowing_is_kept_per_run_label() {
        let answer = json!({
            "overall": {"better": "A", "reason": ""},
            "borrowed_A": {"level": "clear", "what": "x"},
            "borrowed_B": {"level": "none", "what": ""},
        });
        let line = record(Kind::Fidelity, &answer, "front", "front-samples1", true);
        // The second run was Image A.
        assert_eq!(line["borrowed"]["front-samples1"]["level"], "clear");
        assert_eq!(line["borrowed"]["front"]["level"], "none");
        assert_eq!(line["delta"]["overall"], 1);
    }

    #[test]
    fn the_table_averages_both_orders_and_counts_splits() {
        let verdict = |shown: bool, overall: i8| Verdict {
            kind: Kind::Realism,
            character: "x".to_owned(),
            model: "m".to_owned(),
            first: "front".to_owned(),
            second: "front-samples1".to_owned(),
            second_shown_first: shown,
            deltas: BTreeMap::from([("overall".to_owned(), overall)]),
            borrowed: BTreeMap::new(),
            cost: None,
        };
        let verdicts = [verdict(false, 1), verdict(true, 0)];
        let html = table(&verdicts, Kind::Realism);
        assert!(html.contains("<td>A → B</td><td>2</td>"));
        assert!(html.contains("<td class=\"pos\">+0.50</td>"));
        assert!(html.contains("<td>1/1</td>"));
        assert!(table(&verdicts, Kind::Fidelity).is_empty());
    }
}
