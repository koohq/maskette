//! The sample project a new user finds on the first launch: a finished
//! project (a character picture, a head shell front view, its four-view
//! sheet and eye close-up, and design notes, with the chat that made them),
//! so the workspace, the editor and an export can be tried before any AI is
//! set up.
//!
//! `src-tauri/sample/` is an unzipped `.maskette` file. It is zipped again in
//! memory and goes through the normal import, so it lands like any imported
//! project. Its chat and design notes are written in English in
//! `project.json`; [`JAPANESE`] and [`CHINESE`] hold the same lines and the
//! notes in those languages, and the sample takes the language the UI starts
//! in.
//!
//! The pictures: the character was drawn with PixAI (the Tsubaki.3 model and
//! the CUTE LoRA, both licensed for commercial use of their images), and the
//! head shell views were made from it in the app with its default services:
//! GPT Image 2.6 Sunburst for the images, Gemini 3.8 Flash for the analysis
//! and the design notes. The chat keeps the steps and the tool results of
//! that run, in fixed lines. The images are lossy WebP at quality 90, to
//! keep the installer small.
//!
//! The sample comes once. It goes in only when the data folder did not exist
//! when the app started, which tells a first launch from an update of an app
//! already in use (whose folder exists, even with no projects left).
//! `sample-project.json` in the data folder records that it was added.

use std::io::{Cursor, Write};
use std::path::Path;

use serde_json::{Value, json};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::project_file::{self, Kind};
use super::store::{self, Conversation, ConversationStore};
use crate::backend::generation::design_notes::SECTIONS;
use crate::backend::ids;

/// Written into the data folder once the sample is added.
pub const MARKER_NAME: &str = "sample-project.json";

macro_rules! sample_file {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/sample/", $name)) as &[u8],
        )
    };
}

/// The sample's files, by their names inside a `.maskette`.
const FILES: [(&str, &[u8]); 9] = [
    sample_file!("maskette-project.json"),
    sample_file!("project.json"),
    sample_file!("assets/uploads/ref-1-dd7e68-cat-girl.webp"),
    sample_file!("assets/front-1.webp"),
    sample_file!("assets/turnaround-1.webp"),
    sample_file!("assets/eyes-1.webp"),
    sample_file!("generated/f9717312-b909-4d5a-bcec-acb2e7d5b035/candidate-1.webp"),
    sample_file!("generated/c8aaa078-ab28-4570-9b08-053fc916c16c/candidate-1.webp"),
    sample_file!("generated/fb425e96-b312-4af7-9772-0e57278836ac/candidate-1.webp"),
];

/// The sample's title, chat and design notes in one language. The approval
/// needs no line: it is the approve button, which the UI labels in its own
/// language.
struct Texts {
    title: &'static str,
    request: &'static str,
    guessed_views: &'static str,
    front_ready: &'static str,
    /// Also offers the eye close-up.
    turnaround_ready: &'static str,
    eyes_yes: &'static str,
    /// Also offers the design notes.
    eyes_ready: &'static str,
    notes_yes: &'static str,
    notes_ready: &'static str,
    /// The design notes' parts, in the order of [`SECTIONS`].
    notes: [&'static str; 8],
}

impl Texts {
    /// The chat lines, in a fixed order.
    fn lines(&self) -> [&'static str; 8] {
        [
            self.request,
            self.guessed_views,
            self.front_ready,
            self.turnaround_ready,
            self.eyes_yes,
            self.eyes_ready,
            self.notes_yes,
            self.notes_ready,
        ]
    }

    /// The design notes as the card shows them.
    fn sections(&self) -> Value {
        SECTIONS
            .iter()
            .zip(self.notes)
            .map(|(key, text)| json!({"key": key, "text": text}))
            .collect()
    }
}

/// The lines and design notes of `sample/project.json`.
const ENGLISH: Texts = Texts {
    title: "Sample: cat-eared girl",
    request: "Please make a head shell of this girl straight from this picture, without a character design.",
    guessed_views: "There is only a front picture, so the sides and back of the hair will be guessed. Adding a picture tagged Side or Back would pin them down.",
    front_ready: "Here is the head shell front view. Is it OK, or is there anything to change? You can also fine-tune it by hand in the editor.",
    turnaround_ready: "The four-view sheet is ready. Let me know if you'd like any changes.\n\nThe eyes are too small to read in the front view, so I can also make an eye close-up. Shall I make one? It takes one more image generation.",
    eyes_yes: "Yes, please.",
    eyes_ready: "Here is the eye close-up. Would you also like design notes? They put the shapes and colours of each part into words, to back up what the pictures alone may not get across.",
    notes_yes: "Yes, the design notes too, please.",
    notes_ready: "The design notes are ready. You can copy them or save them as text. If anything should change, or you'd like to add something, let me know.",
    notes: [
        "The eyes are a mirrored pair of large, rounded anime eyes. The irises feature a dark teal top that blends into emerald and vibrant yellow-green at the bottom with layered concentric curved patterns and dark pupils. Highlights consist of a prominent round white circle near the top center, two smaller round white dots along the lower corners, and faint tiny specks; they are framed by thick dark brown upper lash lines with an outer spike, fine lower lashes, and a subtle reddish-brown eyelid crease.",
        "The eyebrows are thin, smoothly arched reddish-brown lines positioned above the eyes and partially visible beneath the bangs.",
        "The mouth is closed in a gentle, subtle smile marked by a thin curved line, with no visible teeth or fangs.",
        "The skin is a smooth, fair pale-peach tone with faint soft blush on the cheeks and a tiny dot defining the tip of the nose.",
        "The hair is a bright warm-orange chin-length bob with full, layered bangs tapering across the forehead to the brows, textured wavy layers, and flared, curled tips around the sides and back. A single vertical three-strand braid runs down the hair on the viewer's right from near the ear to the chin.",
        "A pair of upright triangular animal ears sits atop the head, covered in orange fur on the back and outer rim with plush white fur inside.",
        "A faceted five-pointed yellow-gold star clip is pinned in the hair on the viewer's left, and a smaller faceted green star bead is attached at the bottom of the braid on the viewer's right.",
        "",
    ],
};

/// In the UI's words: 「マスク」, 「設定画」, 「正面図」, 「四面図」, 「目の拡大図」,
/// 「デザインメモ」, the tags 「横」「後ろ」. The notes are the analysis model's,
/// written in Japanese in the same run.
const JAPANESE: Texts = Texts {
    title: "サンプル：猫耳の女の子",
    request: "この子のマスクを作ってください。設定画は省いて、この絵から直接お願いします。",
    guessed_views: "正面の絵だけなので、髪の横と後ろは推測で作ります。用途を「横」「後ろ」にした絵を足すと、その形に合わせられます。",
    front_ready: "マスクの正面図ができました。このままでよいか、直したいところがあれば教えてください。エディターで手直しすることもできます。",
    turnaround_ready: "四面図ができました。直したいところがあれば教えてください。\n\n正面図では目が小さく読み取りにくいため、目の拡大図も作れます。作りますか？画像の生成が 1 回増えます。",
    eyes_yes: "はい、お願いします。",
    eyes_ready: "目の拡大図ができました。デザインメモも作りますか？画像だけでは伝わりにくい各部の形や色を、文章で補うメモです。",
    notes_yes: "はい、デザインメモもお願いします。",
    notes_ready: "デザインメモができました。コピーするか、テキストで保存できます。直したいところや、書き足したいことがあれば教えてください。",
    notes: [
        "左右対称の大きな丸いアニメ目です。虹彩は上部の暗い青緑からエメラルド、下部の鮮やかな黄緑色へと変化するグラデーションに同心円状の模様と暗い瞳孔が描かれています。上部中央寄りに大きな丸い白ハイライトが1つ、下部に小さな丸が2つと細かな光の粒があり、目尻に突起のある太い焦げ茶色の上まつげ、細い下まつげ、薄い赤茶色の二重線で縁取られています。",
        "眉は緩やかなアーチを描く細い赤茶色のラインで、前髪の下から部分的に透けて見えています。",
        "口は閉じた状態で、細い上向きの曲線によって控えめな微笑みを浮かべており、歯や八重歯は見えません。",
        "肌は滑らかで色白のペールピーチトーンで、両頬に柔らかなチークが入り、鼻先が小さな点で表現されています。",
        "明るく温かみのあるオレンジ色をした顎丈のウェーブボブで、前髪は眉にかかる長さで自然に分けられ、サイドと後ろの毛先は外ハネにカールしています。向かって右側の耳元から顎にかけて1本の三つ編みが垂直に下りています。",
        "頭頂部に直立した三角形の獣耳があり、背面と外縁はオレンジ色、内側は白いふんわりとした毛で覆われています。",
        "向かって左側の髪に立体的な黄色い星型ヘアクリップが留められており、向かって右側の三つ編みの先端には小さな緑色の星型ビーズが付いています。",
        "",
    ],
};

/// In the UI's words: 头壳, 设定图, 正视图, 四视图, 眼部特写, 设计说明, the tags
/// “侧面”“背面”. The notes are the analysis model's, written in Chinese in
/// the same run.
const CHINESE: Texts = Texts {
    title: "示例：猫耳女孩",
    request: "请帮我做这个女孩的头壳，不用设定图，直接照这张图做。",
    guessed_views: "只有正面的图，头发的侧面和背面会靠推测来做。添加用途设为“侧面”“背面”的图，就能照着它们做。",
    front_ready: "头壳正视图做好了。这样可以吗？有想改的地方请告诉我。也可以在编辑器里手动微调。",
    turnaround_ready: "四视图做好了。如果有想改的地方，请告诉我。\n\n正视图里的眼睛太小，细节不容易看清，我也可以做一张眼部特写。要做一张吗？需要多生成一张图片。",
    eyes_yes: "好的，麻烦了。",
    eyes_ready: "眼部特写做好了。要不要也写一份设计说明？它用文字补充图片不容易表达的各部位形状和颜色。",
    notes_yes: "好的，设计说明也麻烦了。",
    notes_ready: "设计说明写好了。可以复制，也可以保存为文本。如果有想改或想补充的地方，请告诉我。",
    notes: [
        "左右对称的大圆形动漫眼。虹膜从顶部的深青绿色渐变为中部的翠绿、再到底部的明亮黄绿色，内部带有同心圆弧纹路与深色瞳孔。高光包括虹膜偏上方的一颗醒目大白圆点、下方的两颗小白圆点以及数颗细小光斑；眼眶由粗厚的深焦茶色上眼线及眼尾小突起、纤细的下睫毛和浅红褐色的双眼皮弧线构成。",
        "双眉呈柔和微拱的纤细红褐色线条，从刘海后方隐约透出。",
        "嘴部闭合，呈现为一条轻微向上弯曲的细线，带着温和含蓄的微笑，不见牙齿或虎牙。",
        "平滑白皙的浅粉蜜桃肤色，双颊带有柔淡的腮红，鼻尖点有一颗微小的细点。",
        "明亮且温暖的橙色及下巴微卷短发，刘海长及眉毛自然分缕，两侧与后脑勺的发尾均呈蓬松外翘的波浪卷曲；画面右侧耳畔自上而下垂有一根编至发尾的三股细发辫。",
        "头顶生有一对直立的三角形兽耳，耳背和边缘为橙色，内耳覆盖着白色的蓬松绒毛。",
        "画面左侧发丝上佩戴有一枚立体的黄色五角星发夹，画面右侧的三股细发辫末端附有一颗半透明质感的绿色五角星饰珠。",
        "",
    ],
};

fn texts(locale: &str) -> &'static Texts {
    match locale {
        "ja" => &JAPANESE,
        "zh-CN" => &CHINESE,
        _ => &ENGLISH,
    }
}

/// The language the UI starts in on a first launch: the first of the
/// system's preferred languages that Maskette has, else the default. The
/// same rule as the UI's `detectBrowserLocale` (`frontend/src/i18n/locales.ts`).
pub fn ui_locale(preferred: impl IntoIterator<Item = String>) -> &'static str {
    preferred
        .into_iter()
        .find_map(|tag| {
            let tag = tag.to_ascii_lowercase();
            if tag.starts_with("zh") {
                Some("zh-CN")
            } else if tag.starts_with("en") {
                Some("en")
            } else if tag.starts_with("ja") {
                Some("ja")
            } else {
                None
            }
        })
        .unwrap_or(store::DEFAULT_LOCALE)
}

/// Makes the data folder for this launch. When it did not exist yet (the
/// first launch), adds the sample project in `locale`. Only failing to make
/// the folder is an error: a sample that cannot be added is logged and the
/// app starts without it.
pub fn open_data_dir(data_dir: &Path, locale: &str) -> std::io::Result<()> {
    // A folder that cannot be checked is not taken for a new one.
    let is_new = matches!(data_dir.try_exists(), Ok(false));
    std::fs::create_dir_all(data_dir)?;
    if is_new {
        match add_once(data_dir, locale) {
            Ok(Some(id)) => log::info!("added the sample project {id}"),
            Ok(None) => {}
            Err(error) => log::warn!("could not add the sample project: {error}"),
        }
    }
    Ok(())
}

/// Adds the sample project unless the marker says it was added before.
/// Returns the new project's id.
pub fn add_once(data_dir: &Path, locale: &str) -> Result<Option<String>, String> {
    let marker = data_dir.join(MARKER_NAME);
    if marker.try_exists().map_err(|error| error.to_string())? {
        return Ok(None);
    }
    let bundle = bundle().map_err(|error| format!("cannot pack the sample: {error}"))?;
    let mut conversation = project_file::import_project_file(data_dir, Kind::Maskette, &bundle)
        .map_err(|code| format!("cannot import the sample: {code}"))?;
    localize(&mut conversation, store::normalize_locale(locale));
    let id = conversation.id.clone();
    ConversationStore::new(data_dir).insert(conversation);
    let record = json!({"project_id": id, "added_at": ids::now_iso()});
    std::fs::write(&marker, record.to_string())
        .map_err(|error| format!("cannot write {MARKER_NAME}: {error}"))?;
    Ok(Some(id))
}

/// The sample as `.maskette` bytes.
fn bundle() -> zip::result::ZipResult<Vec<u8>> {
    let mut bundle = ZipWriter::new(Cursor::new(Vec::new()));
    // The images are WebP already; deflate would gain nothing.
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, bytes) in FILES {
        bundle.start_file(name, options)?;
        bundle.write_all(bytes)?;
    }
    Ok(bundle.finish()?.into_inner())
}

/// Puts the title, the chat and the design notes (what the user sees and
/// what the assistant reads back) into `locale`.
fn localize(conversation: &mut Conversation, locale: &str) {
    let texts = texts(locale);
    conversation.locale = locale.to_owned();
    conversation.title = texts.title.to_owned();
    for (english, local) in ENGLISH.lines().into_iter().zip(texts.lines()) {
        for event in &mut conversation.events {
            if let Some(Value::String(text)) = event.get_mut("text")
                && text == english
            {
                *text = local.to_owned();
            }
        }
        for message in &mut conversation.llm_messages {
            if let Some(Value::String(content)) = message.get_mut("content") {
                *content = content.replace(english, local);
            }
        }
    }

    let sections = texts.sections();
    if let Some(notes) = conversation.state.design_notes.as_mut() {
        notes["language"] = json!(locale);
        notes["sections"] = sections.clone();
    }
    for event in &mut conversation.events {
        if event.get("type") == Some(&json!("design_notes")) {
            event.insert("language".to_owned(), json!(locale));
            event.insert("sections".to_owned(), sections.clone());
        }
    }
    // The tool result the assistant read the notes from.
    let written: serde_json::Map<String, Value> = SECTIONS
        .iter()
        .zip(texts.notes)
        .map(|(key, text)| ((*key).to_owned(), json!(text)))
        .collect();
    for message in &mut conversation.llm_messages {
        if message["role"] != "tool" || message["name"] != "write_design_notes" {
            continue;
        }
        if let Some(Value::String(content)) = message.get_mut("content")
            && let Ok(mut result) = serde_json::from_str::<Value>(content)
        {
            result["language"] = json!(locale);
            result["notes"] = Value::Object(written.clone());
            *content = result.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Method, StatusCode, header};
    use axum::response::Response;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;
    use crate::backend::agent::tools::reference_path;
    use crate::backend::generation::generated_file_from_url;
    use crate::backend::tests::{TestBackend, authed, json_body};

    fn projects(data_dir: &Path) -> Vec<Conversation> {
        ConversationStore::new(data_dir)
            .list()
            .iter()
            .map(|entry| entry.lock().clone())
            .collect()
    }

    /// One launch of the app on `data_dir`, as `lib.rs` runs it.
    fn launch(data_dir: &Path) {
        open_data_dir(data_dir, "ja").unwrap();
    }

    fn bundled_project() -> Conversation {
        serde_json::from_slice(FILES[1].1).unwrap()
    }

    #[test]
    fn a_first_launch_adds_the_sample_once() {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("app.maskette");
        launch(&data_dir);
        let added = projects(&data_dir);
        assert_eq!(added.len(), 1);
        let marker: Value =
            serde_json::from_slice(&std::fs::read(data_dir.join(MARKER_NAME)).unwrap()).unwrap();
        assert_eq!(marker["project_id"], added[0].id.as_str());

        launch(&data_dir);
        assert_eq!(projects(&data_dir).len(), 1);
    }

    #[test]
    fn a_deleted_sample_does_not_come_back() {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("app.maskette");
        launch(&data_dir);
        let id = projects(&data_dir)[0].id.clone();
        assert!(ConversationStore::new(&data_dir).delete(&id));

        launch(&data_dir);
        assert!(projects(&data_dir).is_empty());
        // Even where the folder test would let it in, the marker keeps it out.
        assert_eq!(add_once(&data_dir, "ja"), Ok(None));
        assert!(projects(&data_dir).is_empty());
    }

    #[test]
    fn an_existing_data_folder_gets_no_sample() {
        // An update of an app already in use: the folder is there, even with
        // no project left in it.
        let data_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(data_dir.path().join(store::DIR_NAME)).unwrap();
        launch(data_dir.path());
        assert!(projects(data_dir.path()).is_empty());
        assert!(!data_dir.path().join(MARKER_NAME).exists());

        let empty = tempfile::tempdir().unwrap();
        launch(empty.path());
        assert!(projects(empty.path()).is_empty());
    }

    #[test]
    fn the_sample_is_a_finished_head_shell_project() {
        let data_dir = tempfile::tempdir().unwrap();
        let id = add_once(data_dir.path(), "en").unwrap().unwrap();
        let project = &projects(data_dir.path())[0];
        assert_eq!(project.id, id);
        assert_eq!(project.status, "idle");
        let state = &project.state;
        assert_eq!(state.references.len(), 1);
        assert_eq!(state.references[0].kind, "front");
        assert_eq!(state.missing_views(), ["side", "back"]);
        assert!(state.analysis.is_some());
        let roles: Vec<(&str, &str)> = state
            .images
            .iter()
            .map(|image| (image.id.as_str(), image.role.as_str()))
            .collect();
        assert_eq!(
            roles,
            [
                ("front-1", "front"),
                ("turnaround-1", "turnaround"),
                ("eyes-1", "eyes")
            ]
        );
        // The design stage was skipped; the head shell front is approved, so
        // the four-view sheet, the eye close-up and the design notes follow
        // from it.
        assert_eq!(state.current_design_id, None);
        assert_eq!(state.approved_design_id, None);
        assert_eq!(state.current_front_id.as_deref(), Some("front-1"));
        assert_eq!(state.approved_front_id.as_deref(), Some("front-1"));
        for image in &state.images[1..] {
            assert_eq!(image.parent_id.as_deref(), Some("front-1"), "{}", image.id);
        }
        let notes = state.design_notes.as_ref().unwrap();
        assert_eq!(notes["front_id"], "front-1");
        assert_eq!(notes["views"], json!(["front", "turnaround", "eyes"]));
        let shown = project
            .events
            .iter()
            .find(|event| event["type"] == "design_notes")
            .unwrap();
        assert_eq!(shown["sections"], notes["sections"]);

        // Every event in order, and each image shown where it was made.
        let seqs: Vec<u64> = project.events.iter().map(store::event_seq).collect();
        assert_eq!(seqs, (1..=project.last_seq()).collect::<Vec<_>>());
        for image in &state.images {
            let shown = project
                .events
                .iter()
                .find(|event| event["type"] == "image" && event["image_id"] == image.id.as_str())
                .unwrap();
            assert_eq!(store::event_seq(shown), image.created_seq);
            assert_eq!(shown["url"], image.url.as_str());
        }
        assert!(
            project
                .events
                .iter()
                .any(|event| event.get("action") == Some(&json!("approve_front")))
        );
        let last_said = project
            .events
            .iter()
            .rfind(|event| event["type"] == "user_message")
            .unwrap();
        assert_eq!(store::event_seq(last_said), state.last_user_seq);

        // Every file is there under the new id, at the size the project says.
        let bundled = bundled_project();
        let text = serde_json::to_string(project).unwrap();
        assert!(!text.contains(&bundled.id));
        for image in &state.images {
            for path in [
                reference_path(data_dir.path(), &image.reference_key).unwrap(),
                generated_file_from_url(data_dir.path(), &image.url).unwrap(),
            ] {
                let decoded = image::open(&path).unwrap();
                assert_eq!(
                    (decoded.width(), decoded.height()),
                    (image.width, image.height),
                    "{}",
                    path.display()
                );
            }
        }
        assert!(
            reference_path(data_dir.path(), &state.references[0].key)
                .unwrap()
                .is_file()
        );
    }

    #[test]
    fn the_sample_speaks_the_language_the_ui_starts_in() {
        // The table follows `sample/project.json`: each English line is there
        // once in the chat and once in what the assistant reads back.
        let bundled = bundled_project();
        assert_eq!(bundled.locale, "en");
        assert_eq!(bundled.title, ENGLISH.title);
        for line in ENGLISH.lines() {
            let shown = bundled
                .events
                .iter()
                .filter(|event| event.get("text") == Some(&json!(line)))
                .count();
            let read = bundled
                .llm_messages
                .iter()
                .filter(|message| {
                    message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains(line))
                })
                .count();
            assert_eq!((shown, read), (1, 1), "{line}");
        }

        for (locale, texts) in [("ja", &JAPANESE), ("zh-CN", &CHINESE), ("en", &ENGLISH)] {
            let data_dir = tempfile::tempdir().unwrap();
            add_once(data_dir.path(), locale).unwrap();
            let project = &projects(data_dir.path())[0];
            assert_eq!(project.locale, locale);
            assert_eq!(project.title, texts.title);
            let shown: Vec<&str> = project
                .events
                .iter()
                .filter_map(|event| event.get("text")?.as_str())
                .filter(|text| !text.is_empty())
                .collect();
            assert_eq!(shown, texts.lines(), "{locale}");
            for line in texts.lines() {
                assert!(
                    project.llm_messages.iter().any(|message| message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains(line))),
                    "{locale}: {line}"
                );
            }
            // The notes, on the card and in what the assistant read back.
            let notes = project.state.design_notes.as_ref().unwrap();
            assert_eq!(notes["language"], locale);
            assert_eq!(notes["sections"], texts.sections(), "{locale}");
            let shown = project
                .events
                .iter()
                .find(|event| event["type"] == "design_notes")
                .unwrap();
            assert_eq!(shown["language"], locale);
            assert_eq!(shown["sections"], texts.sections(), "{locale}");
            let read: Vec<Value> = project
                .llm_messages
                .iter()
                .filter(|message| message["name"] == "write_design_notes")
                .map(|message| serde_json::from_str(message["content"].as_str().unwrap()).unwrap())
                .collect();
            assert_eq!(read.len(), 1);
            assert_eq!(read[0]["ok"], true);
            assert_eq!(read[0]["language"], locale);
            for (key, text) in SECTIONS.iter().zip(texts.notes) {
                assert_eq!(read[0]["notes"][key], text, "{locale}: {key}");
            }
            if locale != "en" {
                let text = serde_json::to_string(project).unwrap();
                for line in ENGLISH.lines().into_iter().chain(ENGLISH.notes) {
                    if !line.is_empty() {
                        assert!(!text.contains(line), "{locale}: {line}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_ui_language_comes_from_the_system_as_in_the_ui() {
        let locale = |tags: &[&str]| ui_locale(tags.iter().map(|tag| (*tag).to_owned()));
        assert_eq!(locale(&["ja-JP"]), "ja");
        assert_eq!(locale(&["fr-FR", "en-GB", "ja-JP"]), "en");
        assert_eq!(locale(&["zh-Hans-CN"]), "zh-CN");
        assert_eq!(locale(&["zh-TW"]), "zh-CN");
        assert_eq!(locale(&["ko-KR"]), "zh-CN");
        assert_eq!(locale(&[]), "zh-CN");
    }

    async fn get(backend: &TestBackend, uri: &str) -> Response {
        let request = authed(Method::GET, uri).body(Body::empty()).unwrap();
        backend.app().oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn the_sample_opens_and_exports_like_any_project() {
        let backend = TestBackend::new(false);
        let id = add_once(backend.dir.path(), "en").unwrap().unwrap();

        let list = json_body(get(&backend, "/api/agent/conversations").await).await;
        assert_eq!(list[0]["id"], id.as_str());
        assert_eq!(list[0]["version_count"], 3);
        let project =
            json_body(get(&backend, &format!("/api/agent/conversations/{id}")).await).await;
        assert_eq!(
            list[0]["thumbnail_url"],
            project["state"]["images"][2]["url"]
        );
        let reference = project["state"]["references"][0]["url"].as_str().unwrap();
        assert_eq!(get(&backend, reference).await.status(), StatusCode::OK);
        for image in project["state"]["images"].as_array().unwrap() {
            let image_id = image["id"].as_str().unwrap();
            for (uri, media_type) in [
                (image["url"].as_str().unwrap().to_owned(), "image/webp"),
                (
                    format!("/api/agent/conversations/{id}/versions/{image_id}/source"),
                    "image/webp",
                ),
                (
                    format!("/api/agent/conversations/{id}/versions/{image_id}/download"),
                    "image/png",
                ),
            ] {
                let response = get(&backend, &uri).await;
                assert_eq!(response.status(), StatusCode::OK, "{uri}");
                assert_eq!(
                    response.headers()[header::CONTENT_TYPE],
                    media_type,
                    "{uri}"
                );
            }
        }

        let exported = get(&backend, &format!("/api/agent/conversations/{id}/export")).await;
        assert_eq!(exported.status(), StatusCode::OK);
        let bytes = exported.into_body().collect().await.unwrap().to_bytes();
        let copy =
            project_file::import_project_file(backend.dir.path(), Kind::Maskette, &bytes).unwrap();
        assert_eq!(copy.state.images.len(), 3);
        assert!(copy.state.design_notes.is_some());
    }
}
