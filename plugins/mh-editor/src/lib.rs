//! Workspace-scoped file access for the system editor.
use base64::{prelude::BASE64_STANDARD, Engine};
use cordis::{Context, Plugin, Service};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
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
/// Formats the webview can preview natively; SVG stays editable text.
fn preview_mime(path: &str) -> Option<&'static str> {
    Some(
        match Path::new(path).extension()?.to_str()?.to_ascii_lowercase().as_str() {
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
fn read(root: &Path, path: &str) -> Result<(String, String), String> {
    decode(bytes(root, path, 2_000_000)?)
}
fn decode(data: Vec<u8>) -> Result<(String, String), String> {
    if data.len() > 2_000_000 {
        return Err("File exceeds 2 MB".into());
    }
    if data.contains(&0) {
        return Err("Двоичные файлы этого типа редактор не открывает".into());
    }
    let has_crlf = data.windows(2).any(|pair| pair == b"\r\n");
    let has_lf = data
        .iter()
        .enumerate()
        .any(|(i, byte)| *byte == b'\n' && (i == 0 || data[i - 1] != b'\r'));
    let has_lone_cr = data
        .iter()
        .enumerate()
        .any(|(i, byte)| *byte == b'\r' && data.get(i + 1) != Some(&b'\n'));
    if (has_crlf && has_lf) || has_lone_cr {
        return Err(
            "Файл содержит смешанные окончания строк; безопасное редактирование недоступно".into(),
        );
    }
    let version = format!("{:x}", Sha256::digest(&data));
    let text = String::from_utf8(data).map_err(|_| "File is not UTF-8")?;
    Ok((text, version))
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
        let binary = data.len() > 2_000_000 || data.contains(&0);
        let mime = preview_mime(path).or_else(|| if binary { sniff(&data) } else { None });
        if let Some(mime) = mime {
            return Ok(serde_json::json!({
                "text": "",
                "version": format!("{:x}", Sha256::digest(&data)),
                "mimeType": mime,
                "data": BASE64_STANDARD.encode(&data),
            }));
        }
        let (text, version) = decode(data)?;
        Ok(serde_json::json!({"text":text,"version":version}))
    }
    pub fn save(
        &self,
        root: &Path,
        path: &str,
        expected: &str,
        text: &str,
    ) -> Result<serde_json::Value, String> {
        if text.len() > 2_000_000 {
            return Err("File exceeds 2 MB".into());
        }
        let _lock = mh_workspace_tools::EDIT_LOCK
            .lock()
            .map_err(|e| e.to_string())?;
        // shortcut: external writers can race the version check; use OS locking or a compare-and-swap filesystem when available.
        let (_, current) = read(root, path)?;
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
        std::fs::write(root.path().join("binary"), [0, 1]).unwrap();
        assert!(service.read(root.path(), "binary").is_err());
        std::fs::write(root.path().join("pixel.PNG"), [0x89, b'P', 0]).unwrap();
        let image = service.read(root.path(), "pixel.PNG").unwrap();
        assert_eq!(image["mimeType"], "image/png");
        assert_eq!(image["data"], "iVAA");
        assert!(service
            .save(root.path(), "pixel.PNG", image["version"].as_str().unwrap(), "")
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
        std::fs::write(root.path().join("mixed.txt"), b"first\r\nsecond\n").unwrap();
        assert!(service.read(root.path(), "mixed.txt").is_err());
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
}
