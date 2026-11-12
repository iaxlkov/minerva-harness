//! Workspace-scoped file access for the system editor.
use base64::{prelude::BASE64_STANDARD, Engine};
use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use cordis::{Context, Plugin, Service};
use encoding_rs::Encoding;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    convert::Infallible,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub struct EditorService;
impl Service for EditorService {
    const NAME: &'static str = "mh.editor";
}
pub struct EditorPlugin;
impl Plugin for EditorPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = std::io::Error;
    fn prepare(&self, _: ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        ctx.try_service::<mh_workspaces::WorkspaceService>()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let _publication = ctx
            .provide(Arc::new(EditorService))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub directory: bool,
}
fn scoped(
    root: &Path,
    path: &str,
    allow_root: bool,
) -> Result<(cap_std::fs::Dir, PathBuf), String> {
    let dir = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|e| e.to_string())?;
    let path = if path.is_empty() && allow_root {
        Path::new(".")
    } else {
        Path::new(path)
    };
    if path.is_absolute()
        || path.to_string_lossy().contains(':')
        || !path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err("Invalid workspace path".into());
    }
    let mut part = PathBuf::new();
    for component in path.components() {
        part.push(component);
        let metadata = dir.symlink_metadata(&part).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("Symlinks are not supported by the editor".into());
        }
    }
    Ok((dir, path.to_path_buf()))
}
const PREVIEW_LIMIT: u64 = 20_000_000;
const TEXT_LIMIT: usize = 2_000_000;
/// Formats the webview can preview natively; SVG stays editable text.
fn preview_mime(path: &str) -> Option<&'static str> {
    Some(
        match Path::new(path)
            .extension()?
            .to_str()?
            .to_ascii_lowercase()
            .as_str()
        {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            "ico" => "image/x-icon",
            "avif" => "image/avif",
            "pdf" => "application/pdf",
            _ => return None,
        },
    )
}
fn bytes(root: &Path, path: &str, limit: u64) -> Result<Vec<u8>, String> {
    let (dir, path) = scoped(root, path, false)?;
    let file = mh_tool_runtime::files::open_regular_file(&dir, &path).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > limit {
        return Err(format!("File exceeds {} MB", limit / 1_000_000));
    }
    Ok(data)
}
/// Recognises previewable binaries whose extension is missing or unusual.
fn sniff(data: &[u8]) -> Option<&'static str> {
    let starts = |magic: &[u8]| data.starts_with(magic);
    if starts(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if starts(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if starts(b"GIF8") {
        Some("image/gif")
    } else if starts(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if starts(b"BM") {
        Some("image/bmp")
    } else if starts(&[0, 0, 1, 0]) {
        Some("image/x-icon")
    } else if data.get(4..12) == Some(b"ftypavif") {
        Some("image/avif")
    } else if starts(b"%PDF") {
        Some("application/pdf")
    } else {
        None
    }
}
fn editable_text(data: &[u8]) -> Result<&str, &'static str> {
    if data.contains(&0) {
        return Err("Файл содержит двоичные данные");
    }
    std::str::from_utf8(data).map_err(|_| "Кодировка файла отличается от UTF-8")
}

fn decode_non_utf8(data: &[u8]) -> Option<(Cow<'_, str>, String)> {
    let (encoding, input, label) = if let Some((encoding, bom_len)) = Encoding::for_bom(data) {
        (encoding, &data[bom_len..], "Кодировка")
    } else {
        if data.contains(&0) || std::str::from_utf8(data).is_ok() {
            return None;
        }
        let mut detector = EncodingDetector::new(Iso2022JpDetection::Deny);
        detector.feed(data, true);
        (
            detector.guess(None, Utf8Detection::Deny),
            data,
            "Предполагаемая кодировка",
        )
    };
    if encoding == encoding_rs::UTF_8 {
        return None;
    }
    let text = encoding.decode_without_bom_handling_and_without_replacement(input)?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n' | '\x0C'))
    {
        return None;
    }
    Some((text, format!("{label}: {}", encoding.name())))
}
impl EditorService {
    pub fn list(&self, root: &Path, path: &str) -> Result<Vec<Entry>, String> {
        let (dir, relative) = scoped(root, path, true)?;
        let selected = dir.open_dir(&relative).map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for entry in selected.entries().map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "Unsupported filename")?;
            if matches!(name.as_str(), ".git" | "node_modules" | "target") {
                continue;
            }
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if !kind.is_file() && !kind.is_dir() {
                continue;
            }
            let item = if path.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", path.trim_end_matches('/'), name)
            };
            entries.push(Entry {
                name,
                path: item,
                directory: kind.is_dir(),
            });
            if entries.len() > 2000 {
                return Err("Directory exceeds 2000 entries".into());
            }
        }
        entries.sort_by(|a, b| {
            b.directory
                .cmp(&a.directory)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    }
    pub fn read(&self, root: &Path, path: &str) -> Result<serde_json::Value, String> {
        let data = bytes(root, path, PREVIEW_LIMIT)?;
        // Content sniffing only for data that cannot be edited as text anyway.
        let binary = data.len() > TEXT_LIMIT || data.contains(&0);
        let mime = preview_mime(path).or_else(|| if binary { sniff(&data) } else { None });
        let version = format!("{:x}", Sha256::digest(&data));
        if let Some(mime) = mime {
            return Ok(serde_json::json!({
                "text": "",
                "version": version,
                "mimeType": mime,
                "data": BASE64_STANDARD.encode(&data),
            }));
        }
        if data.len() > TEXT_LIMIT {
            return Err("File exceeds 2 MB".into());
        }
        Ok(match editable_text(&data) {
            Ok(text) => serde_json::json!({"text":text,"version":version}),
            Err(reason) => {
                let (text, reason) = decode_non_utf8(&data)
                    .unwrap_or_else(|| (String::from_utf8_lossy(&data), reason.into()));
                serde_json::json!({"text":text,"version":version,"readOnlyReason":reason})
            }
        })
    }
    pub fn save(
        &self,
        root: &Path,
        path: &str,
        expected: &str,
        text: &str,
    ) -> Result<serde_json::Value, String> {
        if text.len() > TEXT_LIMIT {
            return Err("File exceeds 2 MB".into());
        }
        let _lock = mh_workspace_tools::EDIT_LOCK
            .lock()
            .map_err(|e| e.to_string())?;
        // shortcut: external writers can race the version check; use OS locking or a compare-and-swap filesystem when available.
        let data = bytes(root, path, TEXT_LIMIT as u64)?;
        editable_text(&data)?;
        let current = format!("{:x}", Sha256::digest(&data));
        if current != expected {
            return Err(
                "Файл изменился на диске. Сохранение отменено; откройте файл заново.".into(),
            );
        }
        let (dir, path) = scoped(root, path, false)?;
        mh_workspace_tools::atomic_write(&dir, &path, text)?;
        Ok(serde_json::json!({"version":format!("{:x}",Sha256::digest(text.as_bytes()))}))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protects_changed_files_and_paths() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("test.txt"), "old").unwrap();
        let service = EditorService;
        let v = service.read(root.path(), "test.txt").unwrap()["version"]
            .as_str()
            .unwrap()
            .to_owned();
        std::fs::write(root.path().join("test.txt"), "external").unwrap();
        assert!(service.save(root.path(), "test.txt", &v, "mine").is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("test.txt")).unwrap(),
            "external"
        );
        assert!(service.read(root.path(), "../outside").is_err());
        let new_version = service.read(root.path(), "test.txt").unwrap()["version"]
            .as_str()
            .unwrap()
            .to_owned();
        let saved = service
            .save(root.path(), "test.txt", &new_version, "edited")
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("test.txt")).unwrap(),
            "edited"
        );
        assert_eq!(
            saved["version"],
            service.read(root.path(), "test.txt").unwrap()["version"]
        );
        std::fs::write(root.path().join("pixel.PNG"), [0x89, b'P', 0]).unwrap();
        let image = service.read(root.path(), "pixel.PNG").unwrap();
        assert_eq!(image["mimeType"], "image/png");
        assert_eq!(image["data"], "iVAA");
        assert!(service
            .save(
                root.path(),
                "pixel.PNG",
                image["version"].as_str().unwrap(),
                ""
            )
            .is_err());
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.resize(3_000_000, 0);
        std::fs::write(root.path().join("screenshot"), &png).unwrap();
        assert_eq!(
            service.read(root.path(), "screenshot").unwrap()["mimeType"],
            "image/png"
        );
        std::fs::write(root.path().join("big.txt"), "x".repeat(2_000_001)).unwrap();
        assert!(service.read(root.path(), "big.txt").is_err());
        std::fs::write(root.path().join("doc.pdf"), b"%PDF").unwrap();
        assert_eq!(
            service.read(root.path(), "doc.pdf").unwrap()["mimeType"],
            "application/pdf"
        );
        let original = b"\xef\xbb\xbffirst\r\n";
        std::fs::write(root.path().join("bom.txt"), original).unwrap();
        let snapshot = service.read(root.path(), "bom.txt").unwrap();
        service
            .save(
                root.path(),
                "bom.txt",
                snapshot["version"].as_str().unwrap(),
                snapshot["text"].as_str().unwrap(),
            )
            .unwrap();
        assert_eq!(
            std::fs::read(root.path().join("bom.txt")).unwrap(),
            original
        );
    }

    #[test]
    fn edits_mixed_line_endings_and_prevents_saving_lossy_previews() {
        let root = tempfile::tempdir().unwrap();
        let service = EditorService;
        let path = "example.unknown";
        let file = root.path().join(path);
        let original = "первая\r\nвторая\nтретья\rчетвёртая";
        std::fs::write(&file, original).unwrap();
        let plain = service.read(root.path(), path).unwrap();
        assert_eq!(plain["text"], original);
        assert!(plain.get("readOnlyReason").is_none());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
        let edited = "первая\r\nвторая\r\nтретья\r\nизменено";
        service
            .save(
                root.path(),
                path,
                plain["version"].as_str().unwrap(),
                edited,
            )
            .unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), edited);

        for original in [b"bin\0\xff".as_slice(), b"legacy \xff\x01"] {
            std::fs::write(&file, original).unwrap();
            let preview = service.read(root.path(), path).unwrap();
            assert_eq!(
                preview["text"].as_str().unwrap(),
                String::from_utf8_lossy(original)
            );
            assert!(preview["readOnlyReason"].is_string());
            assert!(service
                .save(
                    root.path(),
                    path,
                    preview["version"].as_str().unwrap(),
                    "edited"
                )
                .is_err());
            assert_eq!(std::fs::read(&file).unwrap(), original);
        }
    }

    #[test]
    fn detects_legacy_and_bom_encodings_for_read_only_preview() {
        let root = tempfile::tempdir().unwrap();
        let path = "notes.unknown";
        let file = root.path().join(path);
        let service = EditorService;
        let original = "Привет! Это русский текст для проверки кодировки.\r\n";
        let (cp1251, _, invalid) = encoding_rs::WINDOWS_1251.encode(original);
        assert!(!invalid);
        let utf16le = [0xFF, 0xFE]
            .into_iter()
            .chain(original.encode_utf16().flat_map(u16::to_le_bytes))
            .collect::<Vec<_>>();
        let utf16be = [0xFE, 0xFF]
            .into_iter()
            .chain(original.encode_utf16().flat_map(u16::to_be_bytes))
            .collect::<Vec<_>>();
        for (bytes, label) in [
            (cp1251.as_ref(), "Предполагаемая кодировка: windows-1251"),
            (utf16le.as_slice(), "Кодировка: UTF-16LE"),
            (utf16be.as_slice(), "Кодировка: UTF-16BE"),
        ] {
            std::fs::write(&file, bytes).unwrap();
            let result = service.read(root.path(), path).unwrap();
            assert_eq!(result["text"], original);
            assert_eq!(result["readOnlyReason"], label);
            assert!(service
                .save(
                    root.path(),
                    path,
                    result["version"].as_str().unwrap(),
                    "edited"
                )
                .is_err());
            assert_eq!(std::fs::read(&file).unwrap(), bytes);
        }
    }
}
