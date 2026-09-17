//! A `rustic_core` backend over `moatless-s3`.
//!
//! `rustic_core` reaches object storage only through [`ReadBackend`] and
//! [`WriteBackend`], so this adapter is the whole S3 surface a restic-format
//! workspace repository uses. Key layout is restic's own, under a per-task
//! repository prefix:
//!
//! ```text
//! <prefix>/config
//! <prefix>/keys/<id>
//! <prefix>/snapshots/<id>
//! <prefix>/index/<id>
//! <prefix>/data/<first-2-hex-of-id>/<id>
//! ```
//!
//! Both traits are synchronous and `aws-sdk-s3` is async, so every call blocks
//! on a runtime this module owns. See [`runtime`].

use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use bytes::Bytes;
use rustic_core::{
    BytesList, ErrorKind, FileType, Id, ReadBackend, RusticError, RusticResult, WriteBackend,
};
use tracing::warn;

use crate::error::S3Error;
use crate::objects::{
    delete_object, get_object, get_object_range, head_object_size, list_objects_with_size,
    put_object,
};

/// The runtime every backend call blocks on.
///
/// Do not replace with `Handle::block_on`: `rustic_core` runs the backup on its
/// own worker threads, which under `spawn_blocking` are runtime worker threads,
/// where `Handle::block_on` panics.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        #[expect(
            clippy::expect_used,
            reason = "a runtime that cannot be built leaves the backend with no way to reach S3"
        )]
        tokio::runtime::Builder::new_multi_thread()
            // Every call here is network I/O against S3, and the host process
            // runs its own runtime. Defaulting to one worker per core parks a
            // thread per core for the process lifetime after the first call.
            .worker_threads(4)
            .enable_all()
            .thread_name("restic-s3")
            .build()
            .expect("building the restic S3 runtime")
    })
}

/// A restic repository stored under one S3 prefix.
#[derive(Debug, Clone)]
pub struct ResticS3Backend {
    client: aws_sdk_s3::Client,
    bucket: String,
    /// Repository root inside the bucket, with no trailing slash.
    prefix: String,
}

impl ResticS3Backend {
    /// Address the repository rooted at `prefix` in `bucket`.
    ///
    /// `prefix` must be non-empty: an empty one roots every key at `/config`,
    /// `/data/…`, which S3 accepts as a segment with no name.
    pub fn new(client: aws_sdk_s3::Client, bucket: impl Into<String>, prefix: &str) -> Self {
        Self {
            client,
            bucket: bucket.into(),
            prefix: prefix.trim_end_matches('/').to_string(),
        }
    }

    /// The object key holding one repository file.
    fn key(&self, tpe: FileType, id: &Id) -> String {
        let hex = id.to_hex().to_string();
        match tpe {
            FileType::Config => format!("{}/config", self.prefix),
            FileType::Pack => format!("{}/data/{}/{hex}", self.prefix, &hex[0..2]),
            _ => format!("{}/{}/{hex}", self.prefix, tpe.dirname()),
        }
    }

    /// The prefix every file of one type lives under. Packs nest one level
    /// deeper, so this stops at `data/` and the listing is recursive.
    fn type_prefix(&self, tpe: FileType) -> String {
        format!("{}/{}/", self.prefix, tpe.dirname())
    }
}

/// How many times one read is attempted before the operation carrying it
/// fails.
///
/// `aws-sdk-s3` answers `Ok` once the headers arrive, so the client's own
/// retry covers the request and not the response body: a stream breaking
/// part-way through reaches the caller as [`S3Error::ReadBody`], unretried.
const READ_ATTEMPTS: usize = 4;

/// Wait before re-reading, doubled per attempt, so the attempts span ~1.75s.
const READ_RETRY_BACKOFF: Duration = Duration::from_millis(250);

/// Turn an S3 failure into the shape `rustic_core` reports.
fn backend_error(op: &'static str, key: String, err: S3Error) -> Box<RusticError> {
    RusticError::with_source(ErrorKind::Backend, "S3 `{op}` failed for `{key}`.", err)
        .attach_context("op", op)
        .attach_context("key", key)
}

/// Run one read on the backend's runtime, re-reading a body that broke
/// mid-stream up to [`READ_ATTEMPTS`] times.
///
/// Only [`S3Error::ReadBody`] is repeated: every other failure is the
/// request's own answer, already retried, and a repository object is immutable.
fn read_retrying<T, Fut>(
    op: &'static str,
    key: &str,
    mut attempt: impl FnMut() -> Fut,
) -> RusticResult<T>
where
    Fut: Future<Output = Result<T, S3Error>>,
{
    runtime().block_on(async {
        let mut backoff = READ_RETRY_BACKOFF;
        let mut attempts_left = READ_ATTEMPTS;
        loop {
            attempts_left -= 1;
            match attempt().await {
                Ok(value) => return Ok(value),
                Err(err) if attempts_left == 0 || !matches!(err, S3Error::ReadBody(_)) => {
                    return Err(backend_error(op, key.to_string(), err));
                }
                Err(err) => {
                    warn!(op, key, error = %err, attempts_left, "S3 response body broke mid-stream; re-reading");
                    tokio::time::sleep(backoff).await;
                    backoff *= 2;
                }
            }
        }
    })
}

impl ReadBackend for ResticS3Backend {
    fn location(&self) -> String {
        format!("s3:{}/{}", self.bucket, self.prefix)
    }

    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        // `config` is a single key rather than a directory, and its id is not
        // in the name — restic gives it the all-zero id.
        if tpe == FileType::Config {
            let key = self.key(FileType::Config, &Id::default());
            let size = runtime()
                .block_on(head_object_size(&self.client, &self.bucket, &key))
                .map_err(|err| backend_error("head", key, err))?;
            return Ok(match size {
                Some(size) => vec![(Id::default(), u32::try_from(size).unwrap_or(u32::MAX))],
                None => Vec::new(),
            });
        }

        let prefix = self.type_prefix(tpe);
        let objects = read_retrying("list", &prefix, || {
            list_objects_with_size(&self.client, &self.bucket, &prefix)
        })?;

        Ok(objects
            .into_iter()
            .filter_map(|(key, size)| {
                let name = key.rsplit('/').next()?;
                let id = Id::parse_some(name, tpe)?;
                // Clamp rather than drop, like the `Config` branch above. A
                // dropped entry reads to `rustic_core` as a file that was never
                // written, which is how a present pack becomes a missing one.
                Some((id, u32::try_from(size).unwrap_or(u32::MAX)))
            })
            .collect())
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        let key = self.key(tpe, id);
        let bytes = read_retrying("get", &key, || async {
            get_object(&self.client, &self.bucket, &key)
                .await
                .map(|(bytes, _)| bytes)
        })?;
        Ok(Bytes::from(bytes))
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        let key = self.key(tpe, id);
        let bytes = read_retrying("get-range", &key, || {
            get_object_range(
                &self.client,
                &self.bucket,
                &key,
                u64::from(offset),
                u64::from(length),
            )
        })?;
        Ok(Bytes::from(bytes))
    }

    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        self.key(tpe, id)
    }
}

impl WriteBackend for ResticS3Backend {
    // `create` stays the trait default: S3 has no directories to make, and the
    // opendal backend's 256 `data/<hex>/` markers buy nothing here.

    fn write_bytes(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        content: BytesList,
    ) -> RusticResult<()> {
        let key = self.key(tpe, id);
        let mut body = Vec::with_capacity(content.size());
        for chunk in content.slice() {
            body.extend_from_slice(chunk);
        }
        runtime()
            .block_on(put_object(
                &self.client,
                &self.bucket,
                &key,
                body,
                "application/octet-stream",
            ))
            .map_err(|err| backend_error("put", key, err))?;
        Ok(())
    }

    fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> RusticResult<()> {
        let key = self.key(tpe, id);
        runtime()
            .block_on(delete_object(&self.client, &self.bucket, &key))
            .map_err(|err| backend_error("delete", key, err))?;
        Ok(())
    }
}
