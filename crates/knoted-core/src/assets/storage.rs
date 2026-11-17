use super::service::{DomainError, MAX_BYTES, MAX_PIXELS};
use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use uuid::Uuid;

/// Owns a staged file. Tempfile's private mode and Drop remove rejected input.
pub(super) struct Staged {
    file: NamedTempFile,
    pub mime: &'static str,
}

pub(super) fn validate(bytes: &[u8]) -> Result<&'static str, DomainError> {
    if bytes.len() > MAX_BYTES {
        return Err(DomainError::Limit);
    }
    // Decoder panics are malformed input; they must not escape as HTTP 500.
    std::panic::catch_unwind(|| {
        let format = image::guess_format(bytes).map_err(|_| DomainError::InvalidImage)?;
        // Decoders intentionally tolerate missing container tails (PNG stops
        // after pixels; the JPEG wrapper disables strict mode). Require a
        // complete envelope as well as successfully decoded pixel contents.
        let complete = match format {
            ImageFormat::Png => {
                bytes.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 174, 66, 96, 130])
            }
            ImageFormat::Jpeg => bytes.ends_with(&[0xff, 0xd9]),
            ImageFormat::WebP => bytes
                .get(4..8)
                .and_then(|b| b.try_into().ok())
                .is_some_and(|n| u32::from_le_bytes(n) as u64 + 8 == bytes.len() as u64),
            _ => false,
        };
        if !complete {
            return Err(DomainError::InvalidImage);
        }
        if format == ImageFormat::WebP {
            super::webp::check_dimensions(bytes)?;
        }
        let mime = match format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::WebP => "image/webp",
            _ => return Err(DomainError::InvalidImage),
        };
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_PIXELS as u32);
        limits.max_image_height = Some(MAX_PIXELS as u32);
        limits.max_alloc = Some(512 * 1024 * 1024);
        reader.limits(limits.clone());
        let mut decoder = reader.into_decoder().map_err(image_error)?;
        let (w, h) = decoder.dimensions();
        if w == 0 || h == 0 {
            return Err(DomainError::InvalidImage);
        }
        if u64::from(w) * u64::from(h) > MAX_PIXELS {
            return Err(DomainError::Limit);
        }
        // The output is bounded independently of decoder best-effort max_alloc.
        if decoder.total_bytes() > MAX_PIXELS * 8 {
            return Err(DomainError::Limit);
        }
        decoder.set_limits(limits).map_err(image_error)?;
        image::DynamicImage::from_decoder(decoder).map_err(image_error)?;
        Ok(mime)
    })
    .unwrap_or(Err(DomainError::InvalidImage))
}
fn image_error(error: image::ImageError) -> DomainError {
    if matches!(error, image::ImageError::Limits(_)) {
        DomainError::Limit
    } else {
        DomainError::InvalidImage
    }
}

pub(super) fn stage(root: &Path, bytes: &[u8]) -> Result<Staged, DomainError> {
    let mime = validate(bytes)?;
    if !root.is_absolute() {
        return Err(DomainError::Storage);
    }
    // Deployment owns this private directory. Do not follow a replaced root.
    std::fs::DirBuilder::new()
        .recursive(true)
        .create(root)
        .map_err(|_| DomainError::Storage)?;
    if fs::symlink_metadata(root)
        .map_err(|_| DomainError::Storage)?
        .file_type()
        .is_symlink()
    {
        return Err(DomainError::Storage);
    }
    let private = root.join(".staging");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(&private) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(_) => return Err(DomainError::Storage),
        }
    }
    #[cfg(not(unix))]
    fs::create_dir_all(&private).map_err(|_| DomainError::Storage)?;
    let metadata = fs::symlink_metadata(&private).map_err(|_| DomainError::Storage)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(DomainError::Storage);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(DomainError::Storage);
        }
    }
    let mut file = NamedTempFile::new_in(private).map_err(|_| DomainError::Storage)?;
    file.write_all(bytes).map_err(|_| DomainError::Storage)?;
    file.as_file()
        .sync_all()
        .map_err(|_| DomainError::Storage)?;
    Ok(Staged { file, mime })
}

/// Removal is armed until the metadata transaction is ready to commit. A killed process
/// can leave an unregistered UUID file, but DB-gated reads cannot expose it.
pub(super) struct Published {
    path: PathBuf,
    armed: bool,
}
impl Published {
    pub fn retain(mut self) {
        self.armed = false;
    }
}
impl Drop for Published {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}
impl Staged {
    pub fn size(&self) -> Result<i64, DomainError> {
        Ok(self
            .file
            .as_file()
            .metadata()
            .map_err(|_| DomainError::Storage)?
            .len() as i64)
    }
    pub fn publish(self, root: &Path, id: Uuid) -> Result<Published, DomainError> {
        let path = root.join(id.to_string());
        // Atomic no-clobber publication; uploaded names are never accepted.
        self.file
            .persist_noclobber(&path)
            .map_err(|_| DomainError::Storage)?;
        let published = Published { path, armed: true };
        fs::File::open(root)
            .and_then(|f| f.sync_all())
            .map_err(|_| DomainError::Storage)?;
        Ok(published)
    }
}
pub(super) fn read(root: &Path, id: Uuid) -> Result<Vec<u8>, DomainError> {
    let path = root.join(id.to_string());
    let metadata = fs::symlink_metadata(&path).map_err(|_| DomainError::Storage)?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err(DomainError::Storage);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| DomainError::Storage)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DomainError::Storage)?;
    if bytes.len() > MAX_BYTES {
        return Err(DomainError::Storage);
    }
    Ok(bytes)
}
