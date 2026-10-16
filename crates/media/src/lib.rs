//! Images a note refers to, turned into downscaled copies kept in a disposable
//! disk cache.
//!
//! A note names an image only by a *reference* written in the note itself --
//! an `https://` URL, or a path inside the vault (see [`MediaSource::resolve`]).
//! This crate does not decide where originals live and never stores one: there
//! is no attachment store (`tmtroot/architecture.tmt` keeps attachments out of
//! git and leaves their home undecided), and nothing here writes into the
//! vault. It only *reads* what the reference points at and derives a
//! [`Thumbnail`] from it.
//!
//! # What is cached, and where
//!
//! [`MediaCache`] keeps one downscaled copy per `(source, preset)` under a
//! directory the app owns and may lose at any time (it is rebuilt from the
//! originals on demand, so it never goes into the vault or git). The key is a
//! SHA-256 of the source's *identity*:
//!
//! - a remote image: its URL;
//! - a Git LFS pointer file: the object id it carries, so one object is one
//!   entry however many files point at it (and a hit needs no network);
//! - any other vault file: its path, size and modification time (a bare file name that
//!   no path finds -- `ref:+a.png` -- is looked up by Tomet's `VaultLinkIndex`,
//!   the one closest to the note winning); A content hash would
//!   mean reading every original on every list refresh; instead a checkout
//!   that rewrites mtimes costs one redundant re-thumbnail, never a wrong
//!   image.
//!
//! The cache stays under a byte budget by evicting the least recently used
//! files (a hit refreshes the file's mtime). A source that failed to load is
//! remembered for [`FAILURE_TTL`] so a broken URL is not refetched on every
//! refresh -- by the caller, in memory, never on disk: a failure written to
//! the cache outlived the build that caused it, and an app fixed to load the
//! image went on showing the old verdict for a quarter of an hour.
//!
//! # Git LFS
//!
//! A vault may send its images through Git LFS (`*.png filter=lfs` in
//! `.gitattributes`). The app syncs with libgit2, which never runs the LFS
//! filter, so such an image sits in the vault as a ~130-byte *pointer* file
//! and the real bytes stay on the LFS server. [`LfsPointer`] recognizes one,
//! and [`download_lfs_object`] fetches the object it names from the vault's
//! LFS server ([`LfsRemote`], derived from the Git remote the way git-lfs
//! does, authenticated with the same token sync uses): a batch request, then
//! a GET of the address the server gives, with the body checked against the
//! pointer's SHA-256 and size. The object is never stored: the cache keeps
//! only the thumbnail made from it, and the vault is not written to.
//!
//! # Formats
//!
//! Raster thumbnails are stored as JPEG, or PNG when the source has an alpha
//! channel. They are *not* WebP: `image`'s WebP encoder is lossless-only
//! (`image-webp`: "Only supports VP8L lossless encoding"), which would make a
//! photograph's thumbnail larger than the JPEG. SVG is stored and handed back
//! as is, for the UI toolkit to rasterize at whatever size it draws.
//!
//! # Limits
//!
//! Everything read is bounded: bytes fetched or read ([`MAX_REMOTE_BYTES`],
//! [`MAX_LOCAL_BYTES`], [`MAX_SVG_BYTES`]) and the decoder's pixel dimensions
//! and allocation ([`DECODE_MAX_DIMENSION`], [`DECODE_MAX_ALLOC`]), so a note
//! pointing at a huge or hostile file cannot exhaust a phone's memory.
//!
//! # Network
//!
//! Fetching is the caller's: [`Fetcher`] is a trait the app implements with
//! its own HTTP client, which keeps TLS and certificate setup in one place and
//! lets every test here run offline.

mod cache;
mod lfs;
mod source;
mod thumb;

pub use cache::{CacheKey, FAILURE_TTL, MediaCache};
pub use lfs::{LfsAuth, LfsRemote, MAX_LFS_BYTES, download as download_lfs_object};
pub use source::{
    Fetcher, HttpRequest, LfsPointer, MAX_LOCAL_BYTES, MAX_REMOTE_BYTES, MediaSource, Method,
};
pub use thumb::{
    DECODE_MAX_ALLOC, DECODE_MAX_DIMENSION, MAX_SVG_BYTES, Preset, Thumbnail, make_thumbnail,
};
