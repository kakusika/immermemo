//! The thumbnails of the images notes use as icons: produced off the UI
//! thread (see `immermemo_media`), kept decoded and ready for it.
//!
//! Asking for a picture never blocks. [`picture`] answers from the decoded
//! cache or with an empty image, and queues the missing image for a
//! small pool of worker threads; when one finishes, the UI thread is woken,
//! takes in what is ready, and re-renders the lists and the mini card once
//! (a short delay merges a burst of completions into one refresh). The empty
//! image is what the UI already draws a fallback glyph for, so an icon
//! simply appears when its thumbnail does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clru::CLruCache;
use immermemo_media::{CacheKey, FAILURE_TTL, Fetcher, MediaCache, MediaSource, Preset, Thumbnail};
use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode};

use crate::App;

/// How many thumbnails are produced at once.
const WORKERS: usize = 3;

/// How many decoded thumbnails stay in memory (at most ~256KB each).
const DECODED_THUMBS: usize = 96;

/// How many decoded banner and body images stay in memory. They are far
/// larger (up to ~4MB each), and only a few are ever on screen.
const DECODED_WIDE: usize = 8;

/// How much disk the thumbnail cache may use.
pub const CACHE_BUDGET_BYTES: u64 = 64 * 1024 * 1024;

/// How long after a thumbnail is ready the screens are re-rendered; a burst
/// of completions inside it costs one refresh.
const REFRESH_DELAY: Duration = Duration::from_millis(50);

/// How long a request for the same fetch waits for the network.
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

type Wake = Arc<dyn Fn() + Send + Sync>;
type Finished = (Preset, CacheKey, Result<Thumbnail, String>);

struct Job {
    preset: Preset,
    key: CacheKey,
    source: MediaSource,
    cache: Arc<MediaCache>,
    fetcher: Arc<dyn Fetcher>,
    finished: Sender<Finished>,
    wake: Wake,
}

/// The UI thread's side of the thumbnail pipeline.
pub struct Thumbs {
    cache: Arc<MediaCache>,
    fetcher: Arc<dyn Fetcher>,
    jobs: Sender<Job>,
    finished_tx: Sender<Finished>,
    finished_rx: Receiver<Finished>,
    wake: Wake,
    decoded_thumbs: RefCell<CLruCache<CacheKey, slint::Image>>,
    decoded_wide: RefCell<CLruCache<CacheKey, slint::Image>>,
    pending: RefCell<std::collections::HashSet<CacheKey>>,
    failed: RefCell<HashMap<CacheKey, Instant>>,
}

impl Thumbs {
    /// A pipeline that caches under `cache` and fetches with `fetcher`;
    /// `wake` is called (from a worker thread) whenever a job has finished.
    pub fn new(cache: MediaCache, fetcher: Arc<dyn Fetcher>, wake: Wake) -> Self {
        let cache = Arc::new(cache);
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        for index in 0..WORKERS {
            let job_rx = job_rx.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("thumbs-{index}"))
                .spawn(move || {
                    // Ends when `Thumbs` (the only sender) is dropped.
                    while let Ok(job) = job_rx
                        .lock()
                        .map(|rx| rx.recv())
                        .unwrap_or(Err(mpsc::RecvError))
                    {
                        let result = job
                            .cache
                            .produce(&job.source, job.preset, &*job.fetcher)
                            .map_err(|error| format!("{error:#}"));
                        let _ = job.finished.send((job.preset, job.key, result));
                        (job.wake)();
                    }
                });
            if let Err(error) = spawned {
                eprintln!("could not start a thumbnail worker: {error}");
            }
        }
        // Bring a cache left over budget by an earlier run back under it.
        let sweeper = cache.clone();
        let _ = std::thread::Builder::new()
            .name("thumbs-sweep".into())
            .spawn(move || sweeper.sweep());

        let (finished_tx, finished_rx) = mpsc::channel();
        Self {
            cache,
            fetcher,
            jobs,
            finished_tx,
            finished_rx,
            wake,
            decoded_thumbs: RefCell::new(lru(DECODED_THUMBS)),
            decoded_wide: RefCell::new(lru(DECODED_WIDE)),
            pending: RefCell::default(),
            failed: RefCell::default(),
        }
    }

    fn decoded(&self, preset: Preset) -> &RefCell<CLruCache<CacheKey, slint::Image>> {
        match preset {
            Preset::Thumb => &self.decoded_thumbs,
            Preset::Banner | Preset::Body => &self.decoded_wide,
        }
    }

    /// `source` as an image of `preset`'s size if it is decoded and ready;
    /// otherwise an empty image, with it queued unless it is already on its
    /// way or failed recently.
    pub fn picture(&self, source: &MediaSource, preset: Preset) -> slint::Image {
        let Some(key) = CacheKey::new(source, preset) else {
            return slint::Image::default();
        };
        if let Some(image) = self.decoded(preset).borrow_mut().get(&key) {
            return image.clone();
        }
        let failed_recently = self
            .failed
            .borrow()
            .get(&key)
            .is_some_and(|at| at.elapsed() < FAILURE_TTL);
        if !failed_recently && self.pending.borrow_mut().insert(key.clone()) {
            let job = Job {
                preset,
                key: key.clone(),
                source: source.clone(),
                cache: self.cache.clone(),
                fetcher: self.fetcher.clone(),
                finished: self.finished_tx.clone(),
                wake: self.wake.clone(),
            };
            if self.jobs.send(job).is_err() {
                self.pending.borrow_mut().remove(&key);
            }
        }
        slint::Image::default()
    }

    /// Takes in every thumbnail the workers have finished; returns how many.
    pub fn take_finished(&self) -> usize {
        let mut taken = 0;
        while let Ok(finished) = self.finished_rx.try_recv() {
            self.accept(finished);
            taken += 1;
        }
        taken
    }

    fn accept(&self, (preset, key, result): Finished) {
        self.pending.borrow_mut().remove(&key);
        match result {
            Ok(thumb) => match to_image(thumb) {
                Some(image) => {
                    self.failed.borrow_mut().remove(&key);
                    self.decoded(preset).borrow_mut().put(key, image);
                }
                None => {
                    self.failed.borrow_mut().insert(key, Instant::now());
                }
            },
            Err(_) => {
                self.failed.borrow_mut().insert(key, Instant::now());
            }
        }
    }

    /// Waits for every queued thumbnail and takes it in. For tests, which
    /// run no event loop to deliver the wake-up.
    #[cfg(test)]
    pub fn settle(&self) {
        while !self.pending.borrow().is_empty() {
            let Ok(finished) = self.finished_rx.recv_timeout(Duration::from_secs(10)) else {
                panic!("a thumbnail job never finished");
            };
            self.accept(finished);
        }
    }
}

fn lru(capacity: usize) -> CLruCache<CacheKey, slint::Image> {
    CLruCache::new(NonZeroUsize::new(capacity).expect("capacity is non-zero"))
}

/// Where a note's relative image references are resolved: the vault, and the
/// note the reference was written in.
#[derive(Debug, Clone, Default)]
pub struct MediaBase {
    pub vault_dir: PathBuf,
    pub note_rel: PathBuf,
}

impl MediaBase {
    pub fn new(vault_dir: &Path, note_rel: &Path) -> Self {
        Self {
            vault_dir: vault_dir.to_owned(),
            note_rel: note_rel.to_owned(),
        }
    }

    /// No note: nothing resolves, every picture stays empty.
    pub fn none() -> Self {
        Self::default()
    }

    /// `reference`'s image of `preset`'s size, or an empty one while it is
    /// not ready or if the reference leads nowhere readable.
    pub fn picture(&self, reference: &str, preset: Preset) -> slint::Image {
        MediaSource::resolve(reference, &self.vault_dir, &self.note_rel)
            .map(|source| picture(&source, preset))
            .unwrap_or_default()
    }
}

fn to_image(thumb: Thumbnail) -> Option<slint::Image> {
    match thumb {
        Thumbnail::Raster {
            width,
            height,
            rgba,
        } => Some(slint::Image::from_rgba8(
            SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, width, height),
        )),
        Thumbnail::Svg(document) => slint::Image::load_from_svg_data(&document).ok(),
    }
}

/// An HTTPS-only client for [`MediaCache`]: no redirect to plain HTTP, a
/// timeout, a byte cap.
struct UreqFetcher {
    agent: ureq::Agent,
}

impl UreqFetcher {
    fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(FETCH_TIMEOUT))
            .user_agent("immermemo")
            .build()
            .into();
        Self { agent }
    }
}

impl Fetcher for UreqFetcher {
    fn fetch(&self, url: &str, max_bytes: usize) -> anyhow::Result<Vec<u8>> {
        let response = self.agent.get(url).call()?;
        let bytes = response
            .into_body()
            .with_config()
            .limit(max_bytes as u64)
            .read_to_vec()?;
        Ok(bytes)
    }
}

thread_local! {
    static THUMBS: RefCell<Option<Rc<Thumbs>>> = const { RefCell::new(None) };
    static APP: RefCell<Option<slint::Weak<App>>> = const { RefCell::new(None) };
    static REFRESH: Timer = Timer::default();
}

/// Starts the thumbnail pipeline for `app`, caching under `cache_dir`.
pub fn init(app: &App, cache_dir: PathBuf) {
    APP.with(|slot| *slot.borrow_mut() = Some(app.as_weak()));
    // Workers cannot carry the (non-`Send`) app handle, so the wake-up only
    // asks the UI thread to look; it finds the app itself.
    let wake: Wake = Arc::new(|| {
        let _ = slint::invoke_from_event_loop(on_thumbnails_ready);
    });
    let thumbs = Thumbs::new(
        MediaCache::new(cache_dir, CACHE_BUDGET_BYTES),
        Arc::new(UreqFetcher::new()),
        wake,
    );
    install(thumbs);
}

fn install(thumbs: Thumbs) {
    THUMBS.with(|slot| *slot.borrow_mut() = Some(Rc::new(thumbs)));
}

/// `source` as an image of `preset`'s size, or an empty one while it is not
/// ready (or if the pipeline has not been started).
pub fn picture(source: &MediaSource, preset: Preset) -> slint::Image {
    let thumbs = THUMBS.with(|slot| slot.borrow().clone());
    thumbs
        .map(|t| t.picture(source, preset))
        .unwrap_or_default()
}

fn on_thumbnails_ready() {
    let thumbs = THUMBS.with(|slot| slot.borrow().clone());
    let Some(thumbs) = thumbs else { return };
    if thumbs.take_finished() == 0 {
        return;
    }
    REFRESH.with(|timer| {
        if !timer.running() {
            timer.start(TimerMode::SingleShot, REFRESH_DELAY, refresh_screens);
        }
    });
}

/// Re-renders everything that shows a note's icon.
fn refresh_screens() {
    let app = APP.with(|slot| slot.borrow().as_ref().and_then(|weak| weak.upgrade()));
    let session = crate::session::SESSION.with(|cell| cell.borrow().clone());
    let (Some(app), Some(session)) = (app, session) else {
        return;
    };
    session.borrow_mut().directory_views_dirty = true;
    if app.get_active_tab() == 2 && app.get_files_page() == 1 {
        crate::session::refresh_directory_views(&app, &session);
    }
    let body = app.get_body();
    crate::session::update_rendered_body(&app, &session, &body);
    crate::session::update_current_header(&app, &session, &body);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingFetcher {
        calls: AtomicUsize,
        body: Option<Vec<u8>>,
    }

    impl Fetcher for CountingFetcher {
        fn fetch(&self, _url: &str, _max_bytes: usize) -> anyhow::Result<Vec<u8>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.body.clone().ok_or_else(|| anyhow::anyhow!("offline"))
        }
    }

    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 120, 200, 255]));
        let mut out = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn pipeline(dir: &Path, body: Option<Vec<u8>>) -> (Thumbs, Arc<CountingFetcher>) {
        let fetcher = Arc::new(CountingFetcher {
            calls: AtomicUsize::new(0),
            body,
        });
        let thumbs = Thumbs::new(
            MediaCache::new(dir.join("cache"), 1 << 20),
            fetcher.clone(),
            Arc::new(|| {}),
        );
        (thumbs, fetcher)
    }

    #[test]
    fn a_picture_is_empty_until_its_thumbnail_is_produced_then_ready() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("p.png"), png(800, 400)).unwrap();
        let source = MediaSource::resolve("p.png", dir.path(), Path::new("n.tmt")).unwrap();
        let (thumbs, _) = pipeline(dir.path(), None);

        assert_eq!(
            thumbs.picture(&source, Preset::Thumb).size().width,
            0,
            "not ready yet"
        );
        thumbs.settle();
        let ready = thumbs.picture(&source, Preset::Thumb);
        assert_eq!((ready.size().width, ready.size().height), (256, 128));
    }

    #[test]
    fn asking_again_while_a_thumbnail_is_pending_queues_it_once() {
        let dir = tempfile::tempdir().unwrap();
        let (thumbs, fetcher) = pipeline(dir.path(), Some(png(64, 64)));
        let source = MediaSource::Remote("https://example.com/a.png".to_owned());
        for _ in 0..5 {
            thumbs.picture(&source, Preset::Thumb);
        }
        thumbs.settle();
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
        assert!(thumbs.picture(&source, Preset::Thumb).size().width > 0);
    }

    #[test]
    fn a_failed_image_stays_empty_and_is_not_retried_straight_away() {
        let dir = tempfile::tempdir().unwrap();
        let (thumbs, fetcher) = pipeline(dir.path(), None);
        let source = MediaSource::Remote("https://example.com/down.png".to_owned());
        assert_eq!(thumbs.picture(&source, Preset::Thumb).size().width, 0);
        thumbs.settle();
        for _ in 0..3 {
            assert_eq!(thumbs.picture(&source, Preset::Thumb).size().width, 0);
        }
        thumbs.settle();
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_thumbnail_already_on_disk_needs_no_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let source = MediaSource::Remote("https://example.com/a.png".to_owned());
        let (first, fetcher) = pipeline(dir.path(), Some(png(64, 64)));
        first.picture(&source, Preset::Thumb);
        first.settle();
        drop(first);

        let (second, again) = pipeline(dir.path(), None);
        second.picture(&source, Preset::Thumb);
        second.settle();
        assert!(second.picture(&source, Preset::Thumb).size().width > 0);
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
        assert_eq!(again.calls.load(Ordering::SeqCst), 0);
    }

    /// The seam the UI tests use: install a pipeline with a fake fetcher.
    pub(crate) fn install_for_tests(cache_dir: PathBuf) -> Rc<Thumbs> {
        let thumbs = Thumbs::new(
            MediaCache::new(cache_dir, 1 << 20),
            Arc::new(CountingFetcher {
                calls: AtomicUsize::new(0),
                body: None,
            }),
            Arc::new(|| {}),
        );
        install(thumbs);
        THUMBS.with(|slot| slot.borrow().clone()).unwrap()
    }
}
