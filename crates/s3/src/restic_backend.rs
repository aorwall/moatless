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

use std::sync::OnceLock;

use bytes::Bytes;
use rustic_core::{
    BytesList, ErrorKind, FileType, Id, ReadBackend, RusticError, RusticResult, WriteBackend,
};

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

/// Turn an S3 failure into the shape `rustic_core` reports.
fn backend_error(op: &'static str, key: String, err: S3Error) -> Box<RusticError> {
    RusticError::with_source(ErrorKind::Backend, "S3 `{op}` failed for `{key}`.", err)
        .attach_context("op", op)
        .attach_context("key", key)
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
        let objects = runtime()
            .block_on(list_objects_with_size(&self.client, &self.bucket, &prefix))
            .map_err(|err| backend_error("list", prefix, err))?;

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
        let (bytes, _) = runtime()
            .block_on(get_object(&self.client, &self.bucket, &key))
            .map_err(|err| backend_error("get", key, err))?;
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
        let bytes = runtime()
            .block_on(get_object_range(
                &self.client,
                &self.bucket,
                &key,
                u64::from(offset),
                u64::from(length),
            ))
            .map_err(|err| backend_error("get-range", key, err))?;
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
