//! App-owned wallpaper copies under `root/appearance/` (contract §14.2).
//!
//! Files are named `<id>.media` for an opaque lowercase 32-hex id and detected by bytes, never by
//! name. The directory and every file must be plain (no symlink or reparse point). A file is
//! published, without overwriting, and synced before the preferences transaction references it, so
//! a failure can leave only an unreferenced copy. Nothing is collected automatically in 1.3: kept
//! database backups may still reference older files.

use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::fs::{self, File, Metadata};
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{MAX_SOURCE_BYTES, MediaError, decode};
use crate::domain::appearance::AssetId;
use crate::infrastructure::config::AppDirs;

/// Managed files allowed at once.
pub const MAX_FILES: usize = 32;
/// Managed bytes allowed at once.
pub const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

const EXTENSION: &str = "media";

/// Bytes the user picked, validated by magic and awaiting Apply under a chosen id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedMedia {
    pub id: AssetId,
    pub bytes: Arc<Vec<u8>>,
}

/// What [`MediaStore::publish`] did, so a failed Apply can undo exactly its own file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Published {
    /// The file was created by this call (and may be removed again if Apply fails).
    pub created: bool,
}

/// The managed media folder.
#[derive(Debug, Clone)]
pub struct MediaStore {
    dir: PathBuf,
}

fn is_link_like(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn io_error(err: &std::io::Error) -> MediaError {
    if err.kind() == std::io::ErrorKind::NotFound {
        MediaError::Missing
    } else {
        tracing::warn!("managed media i/o failed ({:?})", err.kind());
        MediaError::Io
    }
}

/// Read a user-picked file with at most 16 MiB of actual reads.
///
/// # Errors
/// [`MediaError::SourceTooLarge`] beyond the limit, [`MediaError::Missing`] or [`MediaError::Io`].
pub fn read_source(path: &Path) -> Result<Vec<u8>, MediaError> {
    let file = File::open(path).map_err(|e| io_error(&e))?;
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_SOURCE_BYTES).unwrap_or(u64::MAX) + 1;
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error(&e))?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(MediaError::SourceTooLarge);
    }
    Ok(bytes)
}

fn random_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut id = String::with_capacity(32);
    for salt in 0_u8..2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u8(salt);
        hasher.write_u128(nanos);
        hasher.write_u32(std::process::id());
        let _ = write!(id, "{:016x}", hasher.finish());
    }
    id
}

impl MediaStore {
    #[must_use]
    pub fn new(dirs: &AppDirs) -> Self {
        Self {
            dir: dirs.appearance_dir.clone(),
        }
    }

    fn file_path(&self, id: &AssetId) -> PathBuf {
        self.dir.join(format!("{}.{EXTENSION}", id.as_str()))
    }

    /// The folder, created on first need; refuses anything but a plain directory.
    fn ensure_dir(&self) -> Result<(), MediaError> {
        match fs::symlink_metadata(&self.dir) {
            Ok(meta) if meta.is_dir() && !is_link_like(&meta) => Ok(()),
            Ok(_) => Err(MediaError::Unsafe),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&self.dir)
                    .or_else(|_| fs::create_dir_all(&self.dir))
                    .map_err(|e| io_error(&e))?;
                let meta = fs::symlink_metadata(&self.dir).map_err(|e| io_error(&e))?;
                if meta.is_dir() && !is_link_like(&meta) {
                    Ok(())
                } else {
                    Err(MediaError::Unsafe)
                }
            }
            Err(err) => Err(io_error(&err)),
        }
    }

    /// Managed ids present, with their sizes. A missing folder is empty.
    fn entries(&self) -> Result<Vec<(AssetId, u64)>, MediaError> {
        match fs::symlink_metadata(&self.dir) {
            Ok(meta) if meta.is_dir() && !is_link_like(&meta) => {}
            Ok(_) => return Err(MediaError::Unsafe),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(io_error(&err)),
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(|e| io_error(&e))? {
            let entry = entry.map_err(|e| io_error(&e))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| AssetId::parse(s).ok())
            else {
                continue;
            };
            let meta = fs::symlink_metadata(&path).map_err(|e| io_error(&e))?;
            if meta.is_file() && !is_link_like(&meta) {
                out.push((id, meta.len()));
            }
        }
        out.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        Ok(out)
    }

    /// Managed files and their total size.
    ///
    /// # Errors
    /// [`MediaError::Unsafe`] for a linked folder, otherwise I/O errors.
    pub fn usage(&self) -> Result<(usize, u64), MediaError> {
        let entries = self.entries()?;
        Ok((entries.len(), entries.iter().map(|e| e.1).sum()))
    }

    /// Validate `bytes` by magic and choose the id they will be stored under: an already-managed
    /// copy of the same bytes is reused, otherwise a fresh opaque id is generated.
    ///
    /// # Errors
    /// [`MediaError::Unsupported`], [`MediaError::SourceTooLarge`] or storage errors.
    pub fn stage(&self, bytes: Arc<Vec<u8>>) -> Result<StagedMedia, MediaError> {
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(MediaError::SourceTooLarge);
        }
        if decode::detect(&bytes).is_none() {
            return Err(MediaError::Unsupported);
        }
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        for (id, size) in self.entries()? {
            if size == length && self.read(&id).is_ok_and(|existing| existing == *bytes) {
                return Ok(StagedMedia { id, bytes });
            }
        }
        let taken: Vec<_> = self.entries()?.into_iter().map(|e| e.0).collect();
        loop {
            let id = AssetId::parse(&random_id()).map_err(|_| MediaError::Unsafe)?;
            if !taken.contains(&id) {
                return Ok(StagedMedia { id, bytes });
            }
        }
    }

    /// Write the staged bytes without overwriting anything and sync them, enforcing the 32-file and
    /// 256 MiB limits. Publishing identical bytes that are already managed is a no-op.
    ///
    /// # Errors
    /// [`MediaError::QuotaExceeded`], [`MediaError::Unsafe`] or I/O errors; the existing wallpaper
    /// is untouched in every case.
    pub fn publish(&self, staged: &StagedMedia) -> Result<Published, MediaError> {
        self.ensure_dir()?;
        let path = self.file_path(&staged.id);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && !is_link_like(&meta) => {
                return if self.read(&staged.id)? == *staged.bytes {
                    Ok(Published { created: false })
                } else {
                    Err(MediaError::Unsafe)
                };
            }
            Ok(_) => return Err(MediaError::Unsafe),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(io_error(&err)),
        }
        let (files, total) = self.usage()?;
        let added = u64::try_from(staged.bytes.len()).unwrap_or(u64::MAX);
        if files + 1 > MAX_FILES || total.saturating_add(added) > MAX_TOTAL_BYTES {
            return Err(MediaError::QuotaExceeded);
        }
        let mut temp = tempfile::Builder::new()
            .prefix(".staging-")
            .suffix(".tmp")
            .tempfile_in(&self.dir)
            .map_err(|e| io_error(&e))?;
        temp.write_all(&staged.bytes).map_err(|e| io_error(&e))?;
        temp.as_file().sync_all().map_err(|e| io_error(&e))?;
        temp.persist_noclobber(&path)
            .map_err(|e| io_error(&e.error))?;
        Ok(Published { created: true })
    }

    /// Remove a file this call chain created when the preferences transaction failed. Files that
    /// were already managed are never removed.
    pub fn discard(&self, staged: &StagedMedia, published: Published) {
        if published.created
            && let Err(err) = fs::remove_file(self.file_path(&staged.id))
        {
            tracing::warn!(
                "could not remove an unreferenced wallpaper copy ({:?})",
                err.kind()
            );
        }
    }

    /// The managed bytes for `id`.
    ///
    /// # Errors
    /// [`MediaError::Missing`] when the file is gone (for example after moving only the database),
    /// [`MediaError::Unsafe`] for a link, otherwise I/O errors.
    pub fn read(&self, id: &AssetId) -> Result<Vec<u8>, MediaError> {
        if let Ok(parent) = fs::symlink_metadata(&self.dir)
            && (is_link_like(&parent) || !parent.is_dir())
        {
            return Err(MediaError::Unsafe);
        }
        let path = self.file_path(id);
        let meta = fs::symlink_metadata(&path).map_err(|e| io_error(&e))?;
        if !meta.is_file() || is_link_like(&meta) {
            return Err(MediaError::Unsafe);
        }
        if meta.len() > u64::try_from(MAX_SOURCE_BYTES).unwrap_or(u64::MAX) {
            return Err(MediaError::SourceTooLarge);
        }
        read_source(&path)
    }
}

#[cfg(test)]
mod tests {
    use image::codecs::png::PngEncoder;
    use image::{ExtendedColorType, ImageEncoder};

    use super::*;

    fn png(seed: u8) -> Arc<Vec<u8>> {
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(&[seed, 0, 0, 255], 1, 1, ExtendedColorType::Rgba8)
            .unwrap();
        Arc::new(out)
    }

    fn store() -> (tempfile::TempDir, MediaStore) {
        let root = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(root.path().to_path_buf());
        let store = MediaStore::new(&dirs);
        (root, store)
    }

    #[test]
    fn the_folder_is_created_lazily_and_files_are_opaque_media_ids() {
        let (root, store) = store();
        assert!(!root.path().join("appearance").exists());
        assert_eq!(store.usage().unwrap(), (0, 0));
        assert!(
            !root.path().join("appearance").exists(),
            "reads do not create it"
        );
        let staged = store.stage(png(1)).unwrap();
        assert_eq!(staged.id.as_str().len(), 32);
        assert!(store.publish(&staged).unwrap().created);
        let name = format!("{}.media", staged.id.as_str());
        assert!(root.path().join("appearance").join(name).is_file());
        assert_eq!(store.read(&staged.id).unwrap(), *staged.bytes);
    }

    #[test]
    fn matching_managed_bytes_are_reused_and_publishing_never_clobbers() {
        let (_root, store) = store();
        let first = store.stage(png(1)).unwrap();
        store.publish(&first).unwrap();
        let again = store.stage(png(1)).unwrap();
        assert_eq!(again.id, first.id, "identical bytes reuse the managed copy");
        assert!(!store.publish(&again).unwrap().created);
        assert_eq!(store.usage().unwrap().0, 1);
        let other = store.stage(png(2)).unwrap();
        assert_ne!(other.id, first.id);
        // Another file's bytes under an existing id are refused, and the original is intact.
        let forged = StagedMedia {
            id: first.id.clone(),
            bytes: png(3),
        };
        assert_eq!(store.publish(&forged).unwrap_err(), MediaError::Unsafe);
        assert_eq!(store.read(&first.id).unwrap(), *png(1));
    }

    #[test]
    fn staging_rejects_non_images_and_oversized_sources() {
        let (_root, store) = store();
        assert_eq!(
            store.stage(Arc::new(b"not an image".to_vec())).unwrap_err(),
            MediaError::Unsupported
        );
        let mut huge = (*png(1)).clone();
        huge.resize(MAX_SOURCE_BYTES + 1, 0);
        assert_eq!(
            store.stage(Arc::new(huge)).unwrap_err(),
            MediaError::SourceTooLarge
        );
    }

    #[test]
    fn source_reads_are_bounded_to_16_mib() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("ok.bin");
        let big = dir.path().join("big.bin");
        fs::write(&ok, vec![1_u8; MAX_SOURCE_BYTES]).unwrap();
        fs::write(&big, vec![1_u8; MAX_SOURCE_BYTES + 1]).unwrap();
        assert_eq!(read_source(&ok).unwrap().len(), MAX_SOURCE_BYTES);
        assert_eq!(read_source(&big).unwrap_err(), MediaError::SourceTooLarge);
        assert_eq!(
            read_source(&dir.path().join("nope")).unwrap_err(),
            MediaError::Missing
        );
    }

    #[test]
    fn quota_is_32_files_and_failures_keep_the_current_wallpaper() {
        let (_root, store) = store();
        let first = store.stage(png(0)).unwrap();
        store.publish(&first).unwrap();
        for seed in 1..32_u8 {
            let staged = store.stage(png(seed)).unwrap();
            store.publish(&staged).unwrap();
        }
        assert_eq!(store.usage().unwrap().0, MAX_FILES);
        let extra = store.stage(png(200)).unwrap();
        assert_eq!(
            store.publish(&extra).unwrap_err(),
            MediaError::QuotaExceeded
        );
        assert_eq!(store.usage().unwrap().0, MAX_FILES, "nothing was written");
        assert_eq!(
            store.read(&first.id).unwrap(),
            *png(0),
            "the current copy is intact"
        );
        // Reusing an existing copy needs no quota.
        let reuse = store.stage(png(5)).unwrap();
        assert!(!store.publish(&reuse).unwrap().created);
    }

    #[test]
    fn byte_quota_is_256_mib() {
        let (_root, store) = store();
        let mut big = (*png(1)).clone();
        big.resize(MAX_SOURCE_BYTES, 0);
        for i in 0..16_u8 {
            let mut bytes = big.clone();
            bytes[MAX_SOURCE_BYTES - 1] = i;
            let staged = store.stage(Arc::new(bytes)).unwrap();
            store.publish(&staged).unwrap();
        }
        assert_eq!(
            store.usage().unwrap().1,
            16 * u64::try_from(MAX_SOURCE_BYTES).unwrap()
        );
        let mut bytes = big;
        bytes[MAX_SOURCE_BYTES - 1] = 99;
        let staged = store.stage(Arc::new(bytes)).unwrap();
        assert_eq!(
            store.publish(&staged).unwrap_err(),
            MediaError::QuotaExceeded
        );
    }

    #[test]
    fn discard_removes_only_a_file_this_apply_created() {
        let (_root, store) = store();
        let kept = store.stage(png(1)).unwrap();
        let kept_pub = store.publish(&kept).unwrap();
        let fresh = store.stage(png(2)).unwrap();
        let fresh_pub = store.publish(&fresh).unwrap();
        // A failed Apply of the reused copy leaves it; a failed Apply of a new copy removes it.
        let reused = store.stage(png(1)).unwrap();
        let reused_pub = store.publish(&reused).unwrap();
        store.discard(&reused, reused_pub);
        assert!(store.read(&kept.id).is_ok());
        store.discard(&fresh, fresh_pub);
        assert_eq!(store.read(&fresh.id).unwrap_err(), MediaError::Missing);
        let _ = kept_pub;
    }

    #[test]
    fn a_copied_profile_resolves_under_its_new_root_and_a_moved_database_does_not() {
        let (old_root, store) = store();
        let staged = store.stage(png(7)).unwrap();
        store.publish(&staged).unwrap();
        // Copy the whole data folder: the same id resolves in the new root.
        let new_root = tempfile::tempdir().unwrap();
        fs::create_dir(new_root.path().join("appearance")).unwrap();
        for entry in fs::read_dir(old_root.path().join("appearance")).unwrap() {
            let entry = entry.unwrap();
            fs::copy(
                entry.path(),
                new_root.path().join("appearance").join(entry.file_name()),
            )
            .unwrap();
        }
        let copied = MediaStore::new(&AppDirs::from_root(new_root.path().to_path_buf()));
        assert_eq!(copied.read(&staged.id).unwrap(), *staged.bytes);
        // Only the database moved: the reference stays, the file is missing, and that is reported
        // as `Missing` so the shell falls back to a solid canvas.
        let bare = tempfile::tempdir().unwrap();
        let moved = MediaStore::new(&AppDirs::from_root(bare.path().to_path_buf()));
        assert_eq!(moved.read(&staged.id).unwrap_err(), MediaError::Missing);
    }

    #[cfg(windows)]
    #[test]
    fn a_reparse_point_or_symlinked_folder_is_rejected() {
        let (root, store) = store();
        let staged = store.stage(png(1)).unwrap();
        store.publish(&staged).unwrap();
        let target = tempfile::tempdir().unwrap();
        let link_root = tempfile::tempdir().unwrap();
        let link = link_root.path().join("appearance");
        // Directory symlinks need a privilege on some Windows setups; skip when unavailable.
        if std::os::windows::fs::symlink_dir(target.path(), &link).is_err() {
            return;
        }
        let linked = MediaStore::new(&AppDirs::from_root(link_root.path().to_path_buf()));
        assert_eq!(linked.publish(&staged).unwrap_err(), MediaError::Unsafe);
        assert_eq!(linked.usage().unwrap_err(), MediaError::Unsafe);
        assert_eq!(linked.read(&staged.id).unwrap_err(), MediaError::Unsafe);
        let _ = root;
    }
}
