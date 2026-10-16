//! The disposable on-disk cache of thumbnails.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use anyhow::Context;
use sha2::{Digest, Sha256};

use crate::lfs::LfsRemote;
use crate::source::{Fetcher, MediaSource};
use crate::thumb::{Format, Preset, Thumbnail, make_thumbnail};

/// How long a source that failed to load is left alone before it is tried
/// again. Kept by the caller, in memory: a failure is not written to disk, so
/// a new build of the app tries every image afresh instead of repeating a
/// verdict from the build before it.
pub const FAILURE_TTL: Duration = Duration::from_secs(15 * 60);

/// A sweep runs after this many stores, so the budget holds without walking
/// the cache on every write.
const SWEEP_EVERY_STORES: usize = 32;

/// A sweep evicts down to this fraction of the budget, so it does not run
/// again on the very next store.
const SWEEP_TARGET: f64 = 0.8;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Names one cached thumbnail: a SHA-256 of the preset and the source's
/// [`MediaSource::identity`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey(String);

impl CacheKey {
    /// `None` if the source's identity cannot be determined (a vault file that
    /// vanished).
    pub fn new(source: &MediaSource, preset: Preset) -> Option<Self> {
        let identity = source.identity()?;
        let mut hasher = Sha256::new();
        hasher.update(preset.dir_name().as_bytes());
        hasher.update([0]);
        hasher.update(identity.as_bytes());
        Some(Self(
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        ))
    }

    /// The key as lowercase hex.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A byte-budgeted directory of thumbnails, safe to share between threads.
#[derive(Debug)]
pub struct MediaCache {
    root: PathBuf,
    max_bytes: u64,
    stores: AtomicUsize,
}

impl MediaCache {
    /// A cache under `root` (created on first write) that [`sweep`](Self::sweep)
    /// keeps within `max_bytes`.
    pub fn new(root: PathBuf, max_bytes: u64) -> Self {
        Self {
            root,
            max_bytes,
            stores: AtomicUsize::new(0),
        }
    }

    fn entry_dir(&self, preset: Preset, key: &CacheKey) -> PathBuf {
        self.root.join(preset.dir_name()).join(&key.as_str()[..2])
    }

    fn entry_path(&self, preset: Preset, key: &CacheKey, format: Format) -> PathBuf {
        self.entry_dir(preset, key)
            .join(format!("{}.{}", key.as_str(), format.extension()))
    }

    /// The thumbnail cached under `key`, if any. A hit counts as a use: it
    /// moves the entry to the back of the eviction order. An entry that no
    /// longer decodes is deleted and reported as a miss.
    pub fn lookup(&self, preset: Preset, key: &CacheKey) -> Option<Thumbnail> {
        Format::ALL.into_iter().find_map(|format| {
            let path = self.entry_path(preset, key, format);
            let bytes = fs::read(&path).ok()?;
            match Thumbnail::decode(format, &bytes) {
                Ok(thumb) => {
                    touch(&path);
                    Some(thumb)
                }
                Err(_) => {
                    let _ = fs::remove_file(&path);
                    None
                }
            }
        })
    }

    /// The thumbnail for `source`: the cached one if there is one, otherwise
    /// read (or fetched), shrunk, stored and returned.
    ///
    /// `lfs` is the vault's Git LFS server, used when `source` is a vault file
    /// that turns out to be a pointer to an object kept there.
    ///
    /// Blocking -- reads files and may use the network -- so call it off the
    /// UI thread. Every call that misses the cache tries the source; not
    /// retrying a broken one is the caller's to arrange (see [`FAILURE_TTL`]).
    pub fn produce(
        &self,
        source: &MediaSource,
        preset: Preset,
        fetcher: &dyn Fetcher,
        lfs: Option<&LfsRemote>,
    ) -> anyhow::Result<Thumbnail> {
        let key = CacheKey::new(source, preset).context("the image is not readable")?;
        if let Some(thumb) = self.lookup(preset, &key) {
            return Ok(thumb);
        }
        let thumb = source
            .read(fetcher, lfs)
            .and_then(|bytes| make_thumbnail(&bytes, preset))?;
        // Failing to store only costs a redo next time.
        let _ = self.store(preset, &key, &thumb);
        Ok(thumb)
    }

    fn store(&self, preset: Preset, key: &CacheKey, thumb: &Thumbnail) -> anyhow::Result<()> {
        let (format, bytes) = thumb.encode()?;
        let path = self.entry_path(preset, key, format);
        write_atomically(&path, &bytes)?;
        if self.stores.fetch_add(1, Ordering::Relaxed) % SWEEP_EVERY_STORES
            == SWEEP_EVERY_STORES - 1
        {
            self.sweep();
        }
        Ok(())
    }

    /// Deletes the least recently used entries until the cache is back under
    /// its budget (with some headroom). Cheap when it is already under.
    pub fn sweep(&self) {
        let mut files = Vec::new();
        collect_files(&self.root, &mut files);
        let total: u64 = files.iter().map(|f| f.size).sum();
        if total <= self.max_bytes {
            return;
        }
        files.sort_by_key(|f| f.mtime);
        let target = (self.max_bytes as f64 * SWEEP_TARGET) as u64;
        let mut remaining = total;
        for file in files {
            if remaining <= target {
                break;
            }
            if fs::remove_file(&file.path).is_ok() {
                remaining -= file.size;
            }
        }
    }
}

struct CachedFile {
    path: PathBuf,
    size: u64,
    mtime: SystemTime,
}

fn collect_files(dir: &Path, out: &mut Vec<CachedFile>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(CachedFile {
                path,
                size: meta.len(),
                mtime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
    }
}

/// Marks `path` as just used.
fn touch(path: &Path) {
    let _ = File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now()));
}

/// Writes through a temporary name and renames, so a reader on another
/// thread never sees half a file.
fn write_atomically(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().context("cache path has no parent")?;
    fs::create_dir_all(parent)?;
    let tmp = path.with_extension(format!(
        "tmp{}",
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Serves the same bytes for any URL, or fails, counting its calls.
    struct FakeFetcher {
        body: Option<Vec<u8>>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeFetcher {
        fn serving(body: Vec<u8>) -> Self {
            Self {
                body: Some(body),
                calls: Mutex::default(),
            }
        }

        fn failing() -> Self {
            Self {
                body: None,
                calls: Mutex::default(),
            }
        }

        fn calls(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl Fetcher for FakeFetcher {
        fn fetch(&self, url: &str, _max_bytes: usize) -> anyhow::Result<Vec<u8>> {
            self.calls.lock().unwrap().push(url.to_owned());
            self.body.clone().context("connection refused")
        }
    }

    fn png(width: u32, height: u32, shade: u8) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([shade, 40, 40, 255]));
        let mut out = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// An LFS server holding `object`, counting the requests made to it.
    struct LfsServer {
        object: Vec<u8>,
        requests: Mutex<usize>,
    }

    impl Fetcher for LfsServer {
        fn fetch(&self, _: &str, _: usize) -> anyhow::Result<Vec<u8>> {
            unreachable!("LFS uses request()")
        }

        fn request(&self, request: &crate::source::HttpRequest) -> anyhow::Result<Vec<u8>> {
            *self.requests.lock().unwrap() += 1;
            Ok(match request.method {
                crate::source::Method::Post => serde_json::json!({
                    "objects": [{ "actions": { "download": { "href": "https://github.com/lfs/obj" } } }]
                })
                .to_string()
                .into_bytes(),
                crate::source::Method::Get => self.object.clone(),
            })
        }
    }

    struct NoAuth;

    impl crate::lfs::LfsAuth for NoAuth {
        fn basic_auth(&self) -> Option<(String, String)> {
            None
        }
    }

    fn lfs_remote() -> LfsRemote {
        LfsRemote::new(
            "https://github.com/owner/notes.git",
            std::sync::Arc::new(NoAuth),
        )
        .unwrap()
    }

    /// The pointer file text for `object`.
    fn pointer_text(object: &[u8]) -> String {
        let oid: String = Sha256::digest(object)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize {}\n",
            object.len()
        )
    }

    #[test]
    fn a_pointer_file_is_downloaded_once_and_then_served_from_the_cache_by_object() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let image = png(600, 300, 90);
        std::fs::write(vault.join("one.png"), pointer_text(&image)).unwrap();
        // A second file pointing at the very same object.
        std::fs::write(vault.join("two.png"), pointer_text(&image)).unwrap();
        let cache = cache(&dir, 1 << 20);
        let server = LfsServer {
            object: image,
            requests: Mutex::new(0),
        };
        let resolve = |name| MediaSource::resolve(name, &vault, Path::new("n.tmt")).unwrap();
        let remote = lfs_remote();

        let first = cache
            .produce(&resolve("one.png"), Preset::Thumb, &server, Some(&remote))
            .unwrap();
        assert!(matches!(
            first,
            Thumbnail::Raster {
                width: 256,
                height: 128,
                ..
            }
        ));
        assert_eq!(
            *server.requests.lock().unwrap(),
            2,
            "a batch request and a GET"
        );

        // The same object under another name, and the same file again, cost nothing --
        // not even a remote.
        cache
            .produce(&resolve("two.png"), Preset::Thumb, &server, None)
            .unwrap();
        cache
            .produce(&resolve("one.png"), Preset::Thumb, &server, None)
            .unwrap();
        assert_eq!(*server.requests.lock().unwrap(), 2);
    }

    #[test]
    fn a_pointer_without_an_lfs_server_fails_and_a_bad_object_is_never_used() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let wanted = png(64, 64, 1);
        std::fs::write(vault.join("p.png"), pointer_text(&wanted)).unwrap();
        let source = MediaSource::resolve("p.png", &vault, Path::new("n.tmt")).unwrap();

        let no_remote = cache(&dir, 1 << 20);
        let server = LfsServer {
            object: wanted.clone(),
            requests: Mutex::new(0),
        };
        let error = no_remote
            .produce(&source, Preset::Thumb, &server, None)
            .unwrap_err();
        assert!(
            error.to_string().contains("not linked to a remote"),
            "{error:#}"
        );

        // A server that answers with other bytes of the same length.
        let other_dir = TempDir::new().unwrap();
        let liar = LfsServer {
            object: png(64, 64, 2),
            requests: Mutex::new(0),
        };
        let error = cache(&other_dir, 1 << 20)
            .produce(&source, Preset::Thumb, &liar, Some(&lfs_remote()))
            .unwrap_err();
        assert!(format!("{error:#}").contains("does not match"), "{error:#}");
    }

    fn remote(url: &str) -> MediaSource {
        MediaSource::Remote(url.to_owned())
    }

    fn cache(dir: &TempDir, budget: u64) -> MediaCache {
        MediaCache::new(dir.path().join("cache"), budget)
    }

    #[test]
    fn a_remote_image_is_fetched_once_then_served_from_the_cache() {
        let dir = TempDir::new().unwrap();
        let cache = cache(&dir, 1 << 20);
        let fetcher = FakeFetcher::serving(png(600, 300, 200));
        let source = remote("https://example.com/a.png");

        let first = cache
            .produce(&source, Preset::Thumb, &fetcher, None)
            .unwrap();
        let second = cache
            .produce(&source, Preset::Thumb, &fetcher, None)
            .unwrap();
        assert_eq!(fetcher.calls(), 1);
        for thumb in [&first, &second] {
            assert!(matches!(
                thumb,
                Thumbnail::Raster {
                    width: 256,
                    height: 128,
                    ..
                }
            ));
        }
    }

    #[test]
    fn a_vault_file_is_thumbnailed_and_a_changed_file_gets_a_new_entry() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::write(vault.join("p.png"), png(512, 512, 10)).unwrap();
        let cache = cache(&dir, 1 << 20);
        let fetcher = FakeFetcher::failing();
        let source = MediaSource::resolve("p.png", &vault, Path::new("n.tmt")).unwrap();

        let key = CacheKey::new(&source, Preset::Thumb).unwrap();
        assert!(cache.lookup(Preset::Thumb, &key).is_none());
        cache
            .produce(&source, Preset::Thumb, &fetcher, None)
            .unwrap();
        assert!(cache.lookup(Preset::Thumb, &key).is_some());
        assert_eq!(fetcher.calls(), 0, "a vault file never touches the network");

        std::fs::write(vault.join("p.png"), png(128, 64, 10)).unwrap();
        let changed = CacheKey::new(&source, Preset::Thumb).unwrap();
        assert_ne!(changed, key);
        let thumb = cache
            .produce(&source, Preset::Thumb, &fetcher, None)
            .unwrap();
        assert!(matches!(
            thumb,
            Thumbnail::Raster {
                width: 128,
                height: 64,
                ..
            }
        ));
    }

    #[test]
    fn a_failed_source_leaves_nothing_on_disk_so_a_new_build_retries_it() {
        let dir = TempDir::new().unwrap();
        let cache = cache(&dir, 1 << 20);
        let fetcher = FakeFetcher::failing();
        let source = remote("https://example.com/down.png");

        for attempt in 1..=2 {
            assert!(
                cache
                    .produce(&source, Preset::Thumb, &fetcher, None)
                    .is_err()
            );
            assert_eq!(fetcher.calls(), attempt, "each call tries the source");
        }
        let mut leftovers = Vec::new();
        collect_files(dir.path(), &mut leftovers);
        assert!(leftovers.is_empty(), "no failure marker on disk");
    }

    #[test]
    fn a_source_that_is_not_an_image_fails_and_nothing_is_stored() {
        let dir = TempDir::new().unwrap();
        let cache = cache(&dir, 1 << 20);
        let fetcher = FakeFetcher::serving(b"<html>login required</html>".to_vec());
        let source = remote("https://example.com/page");
        assert!(
            cache
                .produce(&source, Preset::Thumb, &fetcher, None)
                .is_err()
        );
        let mut stored = Vec::new();
        collect_files(dir.path(), &mut stored);
        assert!(stored.is_empty(), "a failure leaves nothing behind");
    }

    #[test]
    fn svg_is_cached_and_returned_as_a_document() {
        let dir = TempDir::new().unwrap();
        let cache = cache(&dir, 1 << 20);
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"/>"#.to_vec();
        let fetcher = FakeFetcher::serving(svg.clone());
        let source = remote("https://example.com/a.svg");
        cache
            .produce(&source, Preset::Thumb, &fetcher, None)
            .unwrap();
        let key = CacheKey::new(&source, Preset::Thumb).unwrap();
        assert_eq!(cache.lookup(Preset::Thumb, &key), Some(Thumbnail::Svg(svg)));
    }

    #[test]
    fn a_corrupt_entry_is_a_miss_and_is_removed() {
        let dir = TempDir::new().unwrap();
        let cache = cache(&dir, 1 << 20);
        let fetcher = FakeFetcher::serving(png(64, 64, 1));
        let source = remote("https://example.com/a.png");
        cache
            .produce(&source, Preset::Thumb, &fetcher, None)
            .unwrap();
        let key = CacheKey::new(&source, Preset::Thumb).unwrap();
        let path = cache.entry_path(Preset::Thumb, &key, Format::Jpeg);
        std::fs::write(&path, b"garbage").unwrap();
        assert!(cache.lookup(Preset::Thumb, &key).is_none());
        assert!(!path.exists());
        // And it is simply produced again.
        cache
            .produce(&source, Preset::Thumb, &fetcher, None)
            .unwrap();
        assert_eq!(fetcher.calls(), 2);
    }

    #[test]
    fn a_sweep_evicts_the_least_recently_used_entries_first() {
        let dir = TempDir::new().unwrap();
        // Roomy while filling, then shrunk by building a second handle on the
        // same directory.
        let filler = cache(&dir, u64::MAX);
        let fetcher = FakeFetcher::serving(png(256, 256, 9));
        let urls: Vec<String> = (0..6)
            .map(|i| format!("https://example.com/{i}.png"))
            .collect();
        let keys: Vec<CacheKey> = urls
            .iter()
            .map(|url| {
                let source = remote(url);
                filler
                    .produce(&source, Preset::Thumb, &fetcher, None)
                    .unwrap();
                CacheKey::new(&source, Preset::Thumb).unwrap()
            })
            .collect();

        // Spread the entries over the last minutes, oldest first, then use
        // the very oldest again.
        for (i, key) in keys.iter().enumerate() {
            let path = filler.entry_path(Preset::Thumb, key, Format::Jpeg);
            let age = Duration::from_secs(600 - 60 * i as u64);
            File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(SystemTime::now() - age)
                .unwrap();
        }
        assert!(filler.lookup(Preset::Thumb, &keys[0]).is_some());

        let size = std::fs::metadata(filler.entry_path(Preset::Thumb, &keys[0], Format::Jpeg))
            .unwrap()
            .len();
        let tight = MediaCache::new(dir.path().join("cache"), size * 3);
        tight.sweep();

        assert!(
            tight.lookup(Preset::Thumb, &keys[0]).is_some(),
            "recently used survives"
        );
        assert!(
            tight.lookup(Preset::Thumb, &keys[1]).is_none(),
            "oldest unused is evicted"
        );
        assert!(
            tight.lookup(Preset::Thumb, &keys[5]).is_some(),
            "newest survives"
        );

        let mut left = Vec::new();
        collect_files(&dir.path().join("cache"), &mut left);
        assert!(left.iter().map(|f| f.size).sum::<u64>() <= size * 3);
    }

    #[test]
    fn a_sweep_under_budget_removes_nothing() {
        let dir = TempDir::new().unwrap();
        let cache = cache(&dir, 1 << 20);
        let fetcher = FakeFetcher::serving(png(64, 64, 1));
        let source = remote("https://example.com/a.png");
        cache
            .produce(&source, Preset::Thumb, &fetcher, None)
            .unwrap();
        cache.sweep();
        let key = CacheKey::new(&source, Preset::Thumb).unwrap();
        assert!(cache.lookup(Preset::Thumb, &key).is_some());
    }

    #[test]
    fn a_vanished_vault_file_has_no_key() {
        let source = MediaSource::Vault(PathBuf::from("/definitely/not/here.png"));
        assert!(CacheKey::new(&source, Preset::Thumb).is_none());
    }
}
