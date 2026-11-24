//! The archive half of `copy_out`: Docker's tar stream read entry by entry into the collector on a blocking thread,
//! with the first error of the client kept aside, so its kind survives the `tar` crate (design, decision 6).

use std::io::{self, Read};
use std::sync::{Arc, Mutex, PoisonError};

use bollard::errors::Error as DockerError;
use futures::{Stream, StreamExt};
use tar::{Archive, EntryType};
use tokio_util::bytes::Bytes;
use tokio_util::io::{StreamReader, SyncIoBridge};

use super::map::{fail, status_code};
use crate::copy::{Collector, EntryKind};
use crate::{ContainerError, CopyOut, ErrorKind};

/// Why reading an archive failed.
#[derive(Debug)]
pub(super) enum ArchiveError {
    /// The stream failed: the first error of the client.
    Docker(DockerError),
    /// The archive could not be read: not a tar, cut short, or the reader panicked.
    Tar(io::Error),
}

/// Reads the archive `stream` into `collector` and returns what it kept and skipped. The stream is read as `tar` asks
/// for it, so the data of a skipped file passes in 32 KiB reads and is never held; a kept file is read into a buffer
/// of its size. Must run on a Tokio runtime, which the reader blocks a thread of.
pub(super) async fn read_archive<S>(stream: S, collector: Collector) -> Result<CopyOut, ArchiveError>
where
    S: Stream<Item = Result<Bytes, DockerError>> + Send + Unpin + 'static,
{
    let slot: Arc<Mutex<Option<DockerError>>> = Arc::default();
    let first = Arc::clone(&slot);
    let stream = stream.map(move |item| {
        item.map_err(|error| {
            first.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(error);
            io::Error::other("the archive stream failed")
        })
    });
    let reader = SyncIoBridge::new(StreamReader::new(stream));
    let read = tokio::task::spawn_blocking(move || collect(reader, collector)).await;
    let read = read.unwrap_or_else(|panic| Err(io::Error::other(format!("the archive reader stopped: {panic}"))));
    // An error of the stream fails the call even where `tar` passed over it: no half result.
    match (slot.lock().unwrap_or_else(PoisonError::into_inner).take(), read) {
        (Some(error), _) => Err(ArchiveError::Docker(error)),
        (None, Ok(out)) => Ok(out),
        (None, Err(error)) => Err(ArchiveError::Tar(error)),
    }
}

/// The result of [`read_archive`] for `copy_out` of `name`; `Ok(None)` for Docker's 404, a missing path unless the
/// container is gone, which the caller inspects again to tell.
pub(super) fn settle(name: &str, read: Result<CopyOut, ArchiveError>) -> Result<Option<CopyOut>, ContainerError> {
    match read {
        Ok(out) => Ok(Some(out)),
        Err(ArchiveError::Docker(error)) if status_code(&error) == Some(404) => Ok(None),
        Err(ArchiveError::Docker(error)) => Err(fail("copy out of", name, &error)),
        Err(ArchiveError::Tar(error)) => Err(ContainerError::new(
            ErrorKind::Other,
            format!("copy out of {name}: Docker's archive could not be read: {error}"),
        )),
    }
}

/// Feeds the entries of the tar `reader` to `collector` until the archive ends or the file limit stops it.
fn collect(reader: impl Read, mut collector: Collector) -> io::Result<CopyOut> {
    let mut archive = Archive::new(reader);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let link = || String::from_utf8_lossy(&entry.link_name_bytes().unwrap_or_default()).into_owned();
        let kind = match entry.header().entry_type() {
            EntryType::Regular | EntryType::Continuous => EntryKind::File,
            EntryType::Directory => EntryKind::Folder,
            EntryType::Symlink => EntryKind::Symlink(link()),
            EntryType::Link => EntryKind::HardLink(link()),
            EntryType::Char | EntryType::Block => EntryKind::Other("a device"),
            EntryType::Fifo => EntryKind::Other("a pipe"),
            EntryType::XGlobalHeader => continue, // a header for the archive, no file
            _ => EntryKind::Other("a special file"),
        };
        let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let (size, mode) = (entry.size(), entry.header().mode()?);
        if !collector.offer(&name, kind, size, mode, |buf| entry.read_exact(buf))? {
            break; // dropping the reader ends the stream
        }
    }
    Ok(collector.finish())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::stream;

    use super::*;
    use crate::{CopyFile, CopyLimits, SkippedFile};

    fn header(kind: EntryType, size: u64, mode: u32) -> tar::Header {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(size);
        header.set_mode(mode);
        header
    }

    /// A header written as it is, with `path` and its checksum.
    fn raw(kind: EntryType, size: u64, mode: u32, path: &str) -> tar::Header {
        let mut header = header(kind, size, mode);
        header.set_path(path).unwrap();
        header.set_cksum();
        header
    }

    fn add(builder: &mut tar::Builder<Vec<u8>>, name: &str, contents: &[u8], mode: u32) {
        builder.append_data(&mut header(EntryType::Regular, contents.len() as u64, mode), name, contents).unwrap();
    }

    /// A PAX record `key=value` with its length in front, as POSIX writes it.
    fn pax_record(key: &str, value: &str) -> Vec<u8> {
        let rest = format!(" {key}={value}\n");
        let mut len = rest.len() + 1;
        while (len.to_string().len() + rest.len()) != len {
            len += 1;
        }
        format!("{len}{rest}").into_bytes()
    }

    /// The archive as one chunk, as Docker might send it.
    fn chunks(tar: Vec<u8>) -> impl Stream<Item = Result<Bytes, DockerError>> + Send + Unpin + 'static {
        stream::iter(vec![Ok(Bytes::from(tar))])
    }

    fn collector(path: &str, limits: CopyLimits) -> Collector {
        Collector::new("job-1", path, limits).unwrap()
    }

    const ROOMY: CopyLimits = CopyLimits { max_files: 100, max_bytes: 1 << 20 };

    #[tokio::test]
    async fn files_links_and_long_names_are_read_from_a_tar() {
        let mut builder = tar::Builder::new(Vec::new());
        builder.append_data(&mut header(EntryType::Directory, 0, 0o755), "out/", io::empty()).unwrap();
        add(&mut builder, "out/a", b"ay", 0o644);
        let long = format!("out/{}/b", "d".repeat(150)); // a GNU long name, as the `tar` crate writes one
        add(&mut builder, &long, b"bee", 0o755);
        let pax_name = format!("out/{}/c", "p".repeat(200)); // a PAX path, as Go's writer makes one
        let record = pax_record("path", &pax_name);
        builder.append(&raw(EntryType::XHeader, record.len() as u64, 0o644, "PaxHeader"), record.as_slice()).unwrap();
        builder.append(&raw(EntryType::Regular, 3, 0o600, "out/short-c"), &b"cee"[..]).unwrap();
        builder.append_link(&mut header(EntryType::Symlink, 0, 0o777), "out/out", "/etc/passwd").unwrap();
        builder.append_link(&mut header(EntryType::Link, 0, 0o644), "out/h", "out/a").unwrap();
        builder.append_data(&mut header(EntryType::Fifo, 0, 0o644), "out/fifo", io::empty()).unwrap();
        let tar = builder.into_inner().unwrap();

        let out = read_archive(chunks(tar), collector("/out", ROOMY)).await.unwrap();
        let with_mode = |path: String, contents: &str, mode| CopyFile { mode, ..CopyFile::new(path, contents) };
        assert_eq!(
            out.files,
            [
                CopyFile::new("/out/a", "ay"),
                with_mode(format!("/{long}"), "bee", 0o755),
                with_mode(format!("/{pax_name}"), "cee", 0o600),
            ]
        );
        assert_eq!(
            out.skipped,
            [
                SkippedFile::new("/out/out", "a symbolic link to /etc/passwd, not followed"),
                SkippedFile::new("/out/h", "a hard link to out/a, not followed"),
                SkippedFile::new("/out/fifo", "a pipe, not a regular file"),
            ]
        );
    }

    #[tokio::test]
    async fn a_big_skipped_entry_streams_past_and_the_next_file_is_kept() {
        const BIG: u64 = 64 << 20;
        // The header of `out/big`, written by hand so its data can come lazily.
        let big = raw(EntryType::Regular, BIG, 0o644, "out/big");
        let mut tail = tar::Builder::new(Vec::new());
        add(&mut tail, "out/small", b"small", 0o644);
        let tail = tail.into_inner().unwrap();

        // The data of `out/big` in 1 MiB chunks, made only when the reader asks; at most a few exist at once.
        let made = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&made);
        let data = stream::iter(0..(BIG >> 20)).map(move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(Bytes::from(vec![0u8; 1 << 20]))
        });
        let stream = stream::iter(vec![Ok(Bytes::copy_from_slice(big.as_bytes()))])
            .chain(data)
            .chain(stream::iter(vec![Ok(Bytes::from(tail))]));

        let out = read_archive(Box::pin(stream), collector("/out", CopyLimits::new(10, 1000))).await.unwrap();
        assert_eq!(out.files, [CopyFile::new("/out/small", "small")]);
        assert_eq!(out.skipped, [SkippedFile::new("/out/big", format!("{BIG} bytes, more than the 1000 left"))]);
        assert_eq!(made.load(Ordering::Relaxed), 64, "streamed through, chunk by chunk");
    }

    #[tokio::test]
    async fn a_stream_error_in_the_middle_of_a_file_fails_the_call_with_its_kind() {
        let mut builder = tar::Builder::new(Vec::new());
        add(&mut builder, "out/a", b"ay", 0o644);
        add(&mut builder, "out/b", &[b'b'; 4096], 0o644);
        let tar = builder.into_inner().unwrap();
        let cut = 1024 + 512 + 100; // after `out/a` and into the data of `out/b`
        let lost = io::Error::new(io::ErrorKind::ConnectionReset, "connection reset");
        let stream = stream::iter(vec![
            Ok(Bytes::copy_from_slice(&tar[..cut])),
            Err(DockerError::IOError { err: lost }),
            Ok(Bytes::copy_from_slice(&tar[cut..])),
        ]);
        let read = read_archive(stream, collector("/out", ROOMY)).await;
        assert!(matches!(read, Err(ArchiveError::Docker(DockerError::IOError { .. }))), "{read:?}");
        let error = settle("job-1", read).unwrap_err();
        assert_eq!(error.kind, ErrorKind::DaemonUnavailable, "{error}");
        assert!(error.message.starts_with("copy out of job-1: "), "{error}");
    }

    #[tokio::test]
    async fn a_404_first_chunk_is_a_missing_path_for_the_caller_to_tell() {
        let missing = DockerError::DockerResponseServerError {
            status_code: 404,
            message: "Could not find the file /missing in container job-1".into(),
        };
        let read = read_archive(stream::iter(vec![Err(missing)]), collector("/missing", ROOMY)).await;
        assert_eq!(settle("job-1", read).unwrap(), None);

        let refused = DockerError::DockerResponseServerError { status_code: 500, message: "boom".into() };
        let read = read_archive(stream::iter(vec![Err(refused)]), collector("/out", ROOMY)).await;
        assert_eq!(settle("job-1", read).unwrap_err().kind, ErrorKind::Other);
    }

    #[tokio::test]
    async fn what_is_not_a_tar_is_other_and_the_file_limit_stops_the_read() {
        let read = read_archive(chunks(vec![b'x'; 700]), collector("/out", ROOMY)).await;
        assert!(matches!(read, Err(ArchiveError::Tar(_))), "{read:?}");
        assert_eq!(settle("job-1", read).unwrap_err().kind, ErrorKind::Other);

        let mut builder = tar::Builder::new(Vec::new());
        for name in ["out/a", "out/b", "out/c"] {
            add(&mut builder, name, b"x", 0o644);
        }
        let mut tar = builder.into_inner().unwrap();
        tar.truncate(1024 + 512); // cut after the header of `out/b`: the reader must stop there
        let out = read_archive(chunks(tar), collector("/out", CopyLimits::new(1, 100))).await.unwrap();
        assert_eq!(out.files.len(), 1);
        assert_eq!(out.skipped, [SkippedFile::new("/out", "more than 1 files; the rest was not read")]);
    }
}
