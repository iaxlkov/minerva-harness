use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use image::ImageFormat;
const MAX_BYTES: usize = 10 * 1024 * 1024;

impl ContextService {
    pub fn put_attachment(
        &self,
        session: &str,
        name: &str,
        mime: &str,
        data: &str,
    ) -> Result<AttachmentRef, String> {
        valid_id(session)?;
        if name.is_empty()
            || name.len() > 255
            || name.contains(['/', '\\'])
            || name.chars().any(char::is_control)
        {
            return Err("attachment name must be a simple filename".into());
        }
        if data.len() > MAX_BYTES.div_ceil(3) * 4 {
            return Err("attachment exceeds 10 MiB".into());
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| "attachment data must be base64")?;
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err("attachment must contain 1 byte to 10 MiB".into());
        }
        let format = match mime {
            "image/png" => ImageFormat::Png,
            "image/jpeg" => ImageFormat::Jpeg,
            "image/gif" => ImageFormat::Gif,
            "image/webp" => ImageFormat::WebP,
            _ => return Err("unsupported raster MIME type".into()),
        };
        if image::guess_format(&bytes).map_err(|e| e.to_string())? != format {
            return Err("attachment MIME does not match its bytes".into());
        }
        let mut reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(160 * 1024 * 1024);
        reader.limits(limits);
        let decoded = reader
            .decode()
            .map_err(|e| format!("invalid raster image: {e}"))?;
        if u64::from(decoded.width()) * u64::from(decoded.height()) > 40_000_000 {
            return Err("attachment exceeds 40 million pixels".into());
        }
        let db = self.db()?;
        let id: String = db
            .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        db.execute(
            "INSERT INTO attachments VALUES (?1,?2,?3,?4,?5)",
            params![id, session, name, mime, bytes],
        )
        .map_err(|e| e.to_string())?;
        Ok(AttachmentRef {
            id,
            name: name.into(),
            mime_type: mime.into(),
            bytes: bytes.len(),
        })
    }
    pub fn get_attachment(&self, session: &str, id: &str) -> Result<AttachmentData, String> {
        valid_id(session)?;
        valid_id(id)?;
        let (name, mime_type, data): (String, String, Vec<u8>) = self
            .db()?
            .query_row(
                "SELECT name,mime,data FROM attachments WHERE session=?1 AND id=?2",
                params![session, id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("attachment not found for session")?;
        Ok(AttachmentData {
            id: id.into(),
            name,
            mime_type,
            bytes: data.len(),
            data: STANDARD.encode(data),
        })
    }
}
