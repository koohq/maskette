//! The sample project a new user finds on the first launch: a finished
//! project (a character picture, a head shell front view and a four-view
//! sheet, with the chat that made them), so the workspace, the editor and an
//! export can be tried before any AI is set up.
//!
//! `src-tauri/sample/` is an unzipped `.maskette` file. It is zipped again in
//! memory and goes through the normal import, so it lands like any imported
//! project. Its chat is written in English in `project.json`; [`JAPANESE`]
//! and [`CHINESE`] hold the same lines, and the sample takes the language the
//! UI starts in.
//!
//! The pictures: the character was drawn with PixAI (the Tsubaki.3 model and
//! the CUTE LoRA, both licensed for commercial use of their images), and the
//! head shell views were generated from it with Nano Banana 2.1. The images
//! are lossy WebP at quality 90, to keep the installer small.
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
const FILES: [(&str, &[u8]); 7] = [
    sample_file!("maskette-project.json"),
    sample_file!("project.json"),
    sample_file!("assets/uploads/ref-1-5dd507-cat-girl.webp"),
    sample_file!("assets/front-1.webp"),
    sample_file!("assets/turnaround-1.webp"),
    sample_file!("generated/ade5e459-cc00-4731-b86d-a98dc8b55699/candidate-1.webp"),
    sample_file!("generated/35b1c0bf-9118-4523-906b-5add51f6493c/candidate-1.webp"),
];

/// The sample's title and chat in one language. The approval needs no line:
/// it is the approve button, which the UI labels in its own language.
struct Texts {
    title: &'static str,
    request: &'static str,
    guessed_views: &'static str,
    front_ready: &'static str,
    turnaround_ready: &'static str,
}

impl Texts {
    /// The chat lines, in a fixed order.
    fn lines(&self) -> [&'static str; 4] {
        [
            self.request,
            self.guessed_views,
            self.front_ready,
            self.turnaround_ready,
        ]
    }
}

/// The lines of `sample/project.json`.
const ENGLISH: Texts = Texts {
    title: "Sample: cat-eared girl",
    request: "Please make a head shell of this girl straight from this picture, without a character design.",
    guessed_views: "There is only a front picture, so the sides and back of the hair will be guessed. Adding a picture tagged Side or Back would pin them down.",
    front_ready: "Here is the head shell front view. Is it OK, or is there anything to change? You can also fine-tune it by hand in the editor.",
    turnaround_ready: "The four-view sheet is ready. Let me know if you'd like any changes.",
};

/// In the UI's words: 「マスク」, 「設定画」, 「正面図」, 「四面図」, the tags 「横」「後ろ」.
const JAPANESE: Texts = Texts {
    title: "サンプル：猫耳の女の子",
    request: "この子のマスクを作ってください。設定画は省いて、この絵から直接お願いします。",
    guessed_views: "正面の絵だけなので、髪の横と後ろは推測で作ります。用途を「横」「後ろ」にした絵を足すと、その形に合わせられます。",
    front_ready: "マスクの正面図ができました。このままでよいか、直したいところがあれば教えてください。エディターで手直しすることもできます。",
    turnaround_ready: "四面図ができました。直したいところがあれば教えてください。",
};

/// In the UI's words: 头壳, 设定图, 正视图, 四视图, the tags “侧面”“背面”.
const CHINESE: Texts = Texts {
    title: "示例：猫耳女孩",
    request: "请帮我做这个女孩的头壳，不用设定图，直接照这张图做。",
    guessed_views: "只有正面的图，头发的侧面和背面会靠推测来做。添加用途设为“侧面”“背面”的图，就能照着它们做。",
    front_ready: "头壳正视图做好了。这样可以吗？有想改的地方请告诉我。也可以在编辑器里手动微调。",
    turnaround_ready: "四视图做好了。如果有想改的地方，请告诉我。",
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

/// Puts the title and the chat (what the user sees and what the assistant
/// reads back) into `locale`.
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
            [("front-1", "front"), ("turnaround-1", "turnaround")]
        );
        // The design stage was skipped; the head shell front is approved, so
        // the four-view sheet follows from it.
        assert_eq!(state.current_design_id, None);
        assert_eq!(state.approved_design_id, None);
        assert_eq!(state.current_front_id.as_deref(), Some("front-1"));
        assert_eq!(state.approved_front_id.as_deref(), Some("front-1"));
        assert_eq!(state.images[1].parent_id.as_deref(), Some("front-1"));

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
        let approval = project
            .events
            .iter()
            .find(|event| event.get("action") == Some(&json!("approve_front")))
            .unwrap();
        assert_eq!(store::event_seq(approval), state.last_user_seq);

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
            if locale != "en" {
                let text = serde_json::to_string(project).unwrap();
                for line in ENGLISH.lines() {
                    assert!(!text.contains(line), "{locale}: {line}");
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
        assert_eq!(list[0]["version_count"], 2);
        let project =
            json_body(get(&backend, &format!("/api/agent/conversations/{id}")).await).await;
        assert_eq!(
            list[0]["thumbnail_url"],
            project["state"]["images"][1]["url"]
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
        assert_eq!(copy.state.images.len(), 2);
    }
}
