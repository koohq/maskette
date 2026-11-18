//! The project file: a zip that carries one whole project between machines.
//!
//! Maskette writes `.maskette` files. It still reads `.vkp` files, the format
//! of Visual Studio Kigurumi's old backend (`app/agent/vkp.py`), so projects
//! made there open here. The two hold the same layout and differ only in the
//! format marker, so the extension says which marker the file must carry:
//!
//!   maskette-project.json  marker of a `.maskette` file (`vsk-project.json` in a `.vkp`),
//!                          with the original conversation / session ids
//!   project.json           the whole conversation (state, references, versions, events, transcript)
//!   assets/...             references/agent/<id>/ - uploads and the clean pixels of every version
//!   generated/...          generated/<session>/   - the public images the version URLs point at
//!
//! Importing always makes a new conversation id (an import is a copy),
//! rewrites the ids inside project.json (every key and URL carries them) and
//! copies the files under the new ids.

use std::io::{Cursor, Read, Write};
use std::path::Path;

use serde_json::{Value, json};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::store::Conversation;
use super::tools::conversation_references;
use crate::backend::generation::{self, join_plain};
use crate::backend::ids;

/// Which project file a name is, by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `.maskette`: what Maskette writes.
    Maskette,
    /// `.vkp`: what Visual Studio Kigurumi wrote. Read only.
    Vkp,
}

impl Kind {
    /// The kind a file name's extension (any case) names, if any.
    pub fn of_file_name(name: &str) -> Option<Self> {
        let extension = Path::new(name).extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("maskette") {
            Some(Self::Maskette)
        } else if extension.eq_ignore_ascii_case("vkp") {
            Some(Self::Vkp)
        } else {
            None
        }
    }

    fn format_name(self) -> &'static str {
        match self {
            Self::Maskette => "maskette-project",
            Self::Vkp => "vsk-project",
        }
    }

    fn manifest_name(self) -> &'static str {
        match self {
            Self::Maskette => "maskette-project.json",
            Self::Vkp => "vsk-project.json",
        }
    }
}

const EXTENSION: &str = "maskette";
const FORMAT_VERSION: u64 = 1;
const PROJECT_NAME: &str = "project.json";
const IMAGE_SUFFIXES: [&str; 4] = ["webp", "png", "jpg", "jpeg"];
const MAX_ENTRIES: usize = 10_000;
const MAX_UNCOMPRESSED_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_IMPORT_BYTES: usize = 200 * 1024 * 1024;

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| IMAGE_SUFFIXES.contains(&extension.to_ascii_lowercase().as_str()))
}

/// Every image below `root`, as (path, `/`-path relative to `root`), sorted.
fn collect_images(root: &Path) -> Vec<(std::path::PathBuf, String)> {
    fn walk(dir: &Path, prefix: &str, found: &mut Vec<(std::path::PathBuf, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let relative = format!("{prefix}{name}");
            if path.is_dir() {
                walk(&path, &format!("{relative}/"), found);
            } else if path.is_file() && is_image(&path) {
                found.push((path, relative));
            }
        }
    }
    let mut found = Vec::new();
    walk(root, "", &mut found);
    found.sort_by(|a, b| a.1.cmp(&b.1));
    found
}

fn zip_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// The conversation, its reference and version pixels and its public images,
/// as `.maskette` bytes. Blocks on the disk.
pub fn build_project_file(data_dir: &Path, conversation: &Conversation) -> Result<Vec<u8>, String> {
    let mut bundle = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let manifest = json!({
        "format": Kind::Maskette.format_name(),
        "version": FORMAT_VERSION,
        "conversation_id": conversation.id,
        "session_id": conversation.state.character_session_id,
        "exported_at": conversation.created_at,
    });
    bundle
        .start_file(Kind::Maskette.manifest_name(), options)
        .map_err(zip_error)?;
    bundle
        .write_all(manifest.to_string().as_bytes())
        .map_err(zip_error)?;
    bundle
        .start_file(PROJECT_NAME, options)
        .map_err(zip_error)?;
    let project = serde_json::to_vec(conversation).map_err(zip_error)?;
    bundle.write_all(&project).map_err(zip_error)?;
    let folders = [
        (
            conversation_references(data_dir, &conversation.id),
            "assets/",
        ),
        (
            generation::generated_root(data_dir).join(&conversation.state.character_session_id),
            "generated/",
        ),
    ];
    for (root, prefix) in folders {
        for (path, relative) in collect_images(&root) {
            let bytes = std::fs::read(&path).map_err(zip_error)?;
            bundle
                .start_file(format!("{prefix}{relative}"), options)
                .map_err(zip_error)?;
            bundle.write_all(&bytes).map_err(zip_error)?;
        }
    }
    Ok(bundle.finish().map_err(zip_error)?.into_inner())
}

fn check_entry_name(name: &str) -> Result<(), &'static str> {
    let unsafe_entry = name.starts_with('/')
        || name.contains('\\')
        || name.split('/').any(|part| part.is_empty() || part == "..")
        || !(name == Kind::Maskette.manifest_name()
            || name == Kind::Vkp.manifest_name()
            || name == PROJECT_NAME
            || name.starts_with("assets/")
            || name.starts_with("generated/"));
    if unsafe_entry {
        Err("unsafe_entry")
    } else {
        Ok(())
    }
}

fn read_entry(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, &'static str> {
    let mut file = archive.by_name(name).map_err(|_| "missing_manifest")?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|_| "not_a_zip")?;
    Ok(bytes)
}

/// Restores a project file of the given kind as a new conversation (a copy;
/// the project it came from is never touched). `Err` is the `detail` for a
/// 400. Blocks on the disk; the caller saves the conversation.
pub fn import_project_file(
    data_dir: &Path,
    kind: Kind,
    data: &[u8],
) -> Result<Conversation, &'static str> {
    let mut archive = ZipArchive::new(Cursor::new(data)).map_err(|_| "not_a_zip")?;
    let mut names = Vec::with_capacity(archive.len());
    let mut total: u64 = 0;
    for index in 0..archive.len() {
        let file = archive.by_index_raw(index).map_err(|_| "not_a_zip")?;
        total = total.saturating_add(file.size());
        names.push(file.name().map_err(|_| "not_a_zip")?.into_owned());
    }
    if names.len() > MAX_ENTRIES || total > MAX_UNCOMPRESSED_BYTES {
        return Err("project_too_large");
    }
    for name in &names {
        check_entry_name(name)?;
    }
    let manifest_name = kind.manifest_name();
    if !names.iter().any(|name| name == manifest_name) {
        // The other kind's marker: a `.vkp` renamed to `.maskette`, or the
        // other way round. The extension says which format to expect.
        return Err(
            if names.iter().any(|name| name.ends_with("-project.json")) {
                "unknown_format"
            } else {
                "missing_manifest"
            },
        );
    }
    let meta: Value = serde_json::from_slice(&read_entry(&mut archive, manifest_name)?)
        .map_err(|_| "invalid_json")?;
    let document: Value = serde_json::from_slice(&read_entry(&mut archive, PROJECT_NAME)?)
        .map_err(|_| "invalid_json")?;
    if meta.get("format").and_then(Value::as_str) != Some(kind.format_name())
        || !document.is_object()
    {
        return Err("unknown_format");
    }

    let text_of =
        |value: Option<&Value>| value.and_then(Value::as_str).unwrap_or_default().to_owned();
    let mut old_id = text_of(meta.get("conversation_id"));
    if old_id.is_empty() {
        old_id = text_of(document.get("id"));
    }
    let mut old_session = text_of(meta.get("session_id"));
    if old_session.is_empty() {
        old_session = text_of(
            document
                .get("state")
                .and_then(|state| state.get("character_session_id")),
        );
    }
    let new_id = ids::uuid4();
    let mut text = document.to_string();
    if !old_id.is_empty() {
        text = text.replace(&old_id, &new_id);
    }
    if !old_session.is_empty() && old_session != old_id {
        text = text.replace(&old_session, &new_id);
    }
    let mut conversation: Conversation =
        serde_json::from_str(&text).map_err(|_| "invalid_project")?;
    conversation.id = new_id.clone();
    conversation.status = "idle".to_owned();

    let assets = conversation_references(data_dir, &new_id);
    let generated = generation::generated_root(data_dir).join(&new_id);
    for name in &names {
        let (root, relative) = if let Some(relative) = name.strip_prefix("assets/") {
            (&assets, relative)
        } else if let Some(relative) = name.strip_prefix("generated/") {
            (&generated, relative)
        } else {
            continue;
        };
        // Defence in depth on top of `check_entry_name`.
        let target = join_plain(root, relative).ok_or("unsafe_entry")?;
        let bytes = read_entry(&mut archive, name)?;
        let write = || -> std::io::Result<()> {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, bytes)
        };
        write().map_err(|error| {
            log::error!("cannot write an imported file: {error}");
            "import_not_saved"
        })?;
    }
    Ok(conversation)
}

/// An ASCII-safe attachment name: the title is a nicety, the id prefix makes
/// it unique.
pub fn download_name(conversation: &Conversation) -> String {
    let safe: String = conversation
        .title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let joined = safe
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let safe = if joined.is_empty() {
        "project".to_owned()
    } else {
        joined
    };
    let short: String = safe.chars().take(40).collect();
    format!(
        "{short}-{}.{EXTENSION}",
        &conversation.id[..8.min(conversation.id.len())]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_must_stay_in_the_bundle_layout() {
        assert!(check_entry_name("project.json").is_ok());
        assert!(check_entry_name("maskette-project.json").is_ok());
        assert!(check_entry_name("vsk-project.json").is_ok());
        assert!(check_entry_name("assets/uploads/a.png").is_ok());
        for name in [
            "/etc/x",
            "assets/../x",
            "assets\\x",
            "other/x",
            "assets//x",
            "other-project.json",
        ] {
            assert_eq!(check_entry_name(name), Err("unsafe_entry"), "{name}");
        }
    }

    #[test]
    fn the_extension_names_the_kind() {
        assert_eq!(Kind::of_file_name("a.maskette"), Some(Kind::Maskette));
        assert_eq!(Kind::of_file_name("A.MASKETTE"), Some(Kind::Maskette));
        assert_eq!(Kind::of_file_name("a.vkp"), Some(Kind::Vkp));
        assert_eq!(Kind::of_file_name("a.VKP"), Some(Kind::Vkp));
        for name in ["a.zip", "a", "maskette", "a.maskette.zip"] {
            assert_eq!(Kind::of_file_name(name), None, "{name}");
        }
    }
}
