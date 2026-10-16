//! Reading session logs without reading them twice.
//!
//! Claude Code and Codex append a JSON record per line to their session
//! files, which reach hundreds of megabytes; a month of them runs to
//! gigabytes. What a file parsed to is cached by path, kept while the file
//! keeps its inode, length and modification time, and a file that only grew
//! is read on from where reading stopped (the bytes before that point still
//! hashing the same), with the parser's state as it was there. The cache is
//! also kept on disk, one record per file, so a relaunch reads only what
//! changed. The idea is `sessions::claude::transcript`'s summary cache; the
//! reading rules (a record over 1 MiB is skipped, a line still being written
//! is read but not resumed from) are OpenUsage's `JSONLStreamingReader` and
//! `IncrementalJSONLScanner` (MIT, <https://github.com/robinebers/openusage>).
//!
//! Codex compresses the rollouts it is done with to `.jsonl.zst`; those are
//! read through a zstd decoder, whole, as they are no longer written to.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::app_state::atomic_write;

/// How much is read at a time.
const CHUNK_BYTES: usize = 64 * 1024;
/// A record longer than this is skipped, so one runaway line can't take the
/// memory with it.
pub const MAX_RECORD_BYTES: usize = 1024 * 1024;
/// How much of a file before the resume point is hashed to tell a file that
/// grew from one rewritten.
const FINGERPRINT_BYTES: u64 = 4096;
/// Files parsed at once.
const PARSE_THREADS: usize = 8;
/// Bumped when the on-disk layout changes.
const FORMAT_VERSION: u32 = 1;

/// Reads the records of one kind of log, line by line, keeping whatever
/// state it needs between lines. A file is read by one parser from its
/// start, or by a copy of the state it had where reading stopped.
pub trait Parser: Clone + Default + Send + Sync + Serialize + DeserializeOwned {
    type Item: Clone + Send + Sync + Serialize + DeserializeOwned;

    /// Takes in one line (without its newline), adding what it yields.
    fn line(&mut self, line: &[u8], items: &mut Vec<Self::Item>);
}

/// A log file found on disk, with what tells whether it changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFile {
    pub path: PathBuf,
    pub inode: u64,
    pub size: u64,
    /// Nanoseconds since the epoch.
    pub modified: i128,
}

impl LogFile {
    /// The file at `path`, if it is a regular file (not a link).
    pub fn at(path: &Path) -> Option<Self> {
        let metadata = fs::symlink_metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        Some(Self {
            path: path.to_path_buf(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified: nanos(metadata.modified().ok()?),
        })
    }
}

pub fn nanos(time: SystemTime) -> i128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_nanos() as i128,
        Err(before) => -(before.duration().as_nanos() as i128),
    }
}

/// The suffix of a log compressed with zstd.
const COMPRESSED: &str = ".jsonl.zst";

fn is_compressed(path: &Path) -> bool {
    path.as_os_str()
        .as_encoded_bytes()
        .ends_with(COMPRESSED.as_bytes())
}

/// Every `*.jsonl` and `*.jsonl.zst` regular file under `dir`, recursively,
/// sorted by path. Links aren't followed, except `dir` itself. Empty when
/// `dir` can't be read.
pub fn jsonl_files(dir: &Path) -> Vec<LogFile> {
    let mut files = Vec::new();
    let dir = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    walk(&dir, &mut |path| {
        if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
            || is_compressed(path)
        {
            files.extend(LogFile::at(path));
        }
    });
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// Calls `visit` with every file under `dir`, recursively, skipping links.
pub fn walk(dir: &Path, visit: &mut dyn FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            walk(&entry.path(), visit);
        } else if kind.is_file() {
            visit(&entry.path());
        }
    }
}

/// What reading a file from some point found.
struct Reading<P: Parser> {
    items: Vec<P::Item>,
    parser: P,
    /// The end of the last whole line read.
    offset: u64,
    /// The file ended on a newline, so reading can go on from `offset`.
    at_line_end: bool,
}

/// Reads `path` from `start` up to `end` (or its end), passing each line to
/// `parser`. A record over [`MAX_RECORD_BYTES`] is skipped whole; a last
/// line without a newline is read, but leaves `at_line_end` false.
fn read_from<P: Parser>(
    path: &Path,
    start: u64,
    end: Option<u64>,
    parser: P,
) -> std::io::Result<Reading<P>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    read_lines(file, start, end, parser)
}

/// Reads the compressed log at `path`, whole.
fn read_compressed<P: Parser>(path: &Path, parser: P) -> std::io::Result<Reading<P>> {
    let decoder = ruzstd::decoding::StreamingDecoder::new(File::open(path)?)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;
    read_lines(decoder, 0, None, parser)
}

/// [`read_from`]'s reading, of `source` positioned at `start`.
fn read_lines<P: Parser>(
    mut source: impl Read,
    start: u64,
    end: Option<u64>,
    mut parser: P,
) -> std::io::Result<Reading<P>> {
    let mut items = Vec::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut discarding = false;
    let mut position = start;
    let mut offset = start;
    let mut at_line_end = true;
    let mut chunk = vec![0_u8; CHUNK_BYTES];
    loop {
        let want = match end {
            Some(end) => CHUNK_BYTES
                .min(usize::try_from(end.saturating_sub(position)).unwrap_or(CHUNK_BYTES)),
            None => CHUNK_BYTES,
        };
        if want == 0 {
            break;
        }
        let count = source.read(&mut chunk[..want])?;
        if count == 0 {
            break;
        }
        let mut rest = &chunk[..count];
        while !rest.is_empty() {
            let newline = rest.iter().position(|byte| *byte == b'\n');
            let fragment = &rest[..newline.unwrap_or(rest.len())];
            if !discarding {
                if fragment.len() > MAX_RECORD_BYTES - pending.len() {
                    discarding = true;
                    pending = Vec::new();
                } else {
                    pending.extend_from_slice(fragment);
                }
            }
            position += fragment.len() as u64;
            match newline {
                Some(at) => {
                    if !discarding && !pending.is_empty() {
                        parser.line(&pending, &mut items);
                    }
                    pending.clear();
                    discarding = false;
                    position += 1;
                    offset = position;
                    at_line_end = true;
                    rest = &rest[at + 1..];
                }
                None => {
                    if !fragment.is_empty() {
                        at_line_end = false;
                    }
                    rest = &[];
                }
            }
        }
    }
    if discarding {
        at_line_end = false;
    } else if !pending.is_empty() {
        at_line_end = false;
        parser.line(&pending, &mut items);
    }
    Ok(Reading {
        items,
        parser,
        offset,
        at_line_end,
    })
}

/// FNV-1a over the [`FINGERPRINT_BYTES`] before `offset`.
fn fingerprint(path: &Path, offset: u64) -> Option<u64> {
    let start = offset.saturating_sub(FINGERPRINT_BYTES);
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = vec![0_u8; usize::try_from(offset - start).ok()?];
    file.read_exact(&mut bytes).ok()?;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    Some(hash)
}

/// Where reading a file can go on from.
#[derive(Clone, Serialize, Deserialize)]
#[serde(bound = "P: Parser")]
struct Resume<P: Parser> {
    offset: u64,
    fingerprint: u64,
    parser: P,
}

/// What a file parsed to, as of its inode, length and modification time.
#[derive(Clone, Serialize, Deserialize)]
#[serde(bound = "P: Parser")]
struct Parsed<P: Parser> {
    inode: u64,
    size: u64,
    modified: i128,
    items: Arc<Vec<P::Item>>,
    resume: Option<Resume<P>>,
}

impl<P: Parser> Parsed<P> {
    fn is_of(&self, file: &LogFile) -> bool {
        self.inode == file.inode && self.size == file.size && self.modified == file.modified
    }
}

/// Reads `file` whole, or on from `cached` when the file only grew since.
/// `None` when it can't be read.
fn parse<P: Parser>(file: &LogFile, cached: Option<&Parsed<P>>) -> Option<Parsed<P>> {
    if is_compressed(&file.path) {
        let read = read_compressed(&file.path, P::default()).ok()?;
        return Some(Parsed {
            inode: file.inode,
            size: file.size,
            modified: file.modified,
            items: Arc::new(read.items),
            resume: None,
        });
    }
    if let Some(parsed) = cached.and_then(|cached| parse_on(file, cached)) {
        return Some(parsed);
    }
    let read = read_from(&file.path, 0, None, P::default()).ok()?;
    let resume = (read.at_line_end && read.offset == file.size)
        .then(|| fingerprint(&file.path, read.offset))
        .flatten()
        .map(|fingerprint| Resume {
            offset: read.offset,
            fingerprint,
            parser: read.parser,
        });
    Some(Parsed {
        inode: file.inode,
        size: file.size,
        modified: file.modified,
        items: Arc::new(read.items),
        resume,
    })
}

/// Reads on from where `cached` stopped, if `file` is the same file grown
/// from it: same inode, longer, and the bytes before the resume point
/// unchanged. Only a read that ends on a newline at the end counts.
fn parse_on<P: Parser>(file: &LogFile, cached: &Parsed<P>) -> Option<Parsed<P>> {
    let resume = cached.resume.as_ref()?;
    if cached.inode != file.inode || file.size <= cached.size || resume.offset > cached.size {
        return None;
    }
    if fingerprint(&file.path, resume.offset)? != resume.fingerprint {
        return None;
    }
    let read = read_from(
        &file.path,
        resume.offset,
        Some(file.size),
        resume.parser.clone(),
    )
    .ok()?;
    if !read.at_line_end || read.offset != file.size || read.offset <= resume.offset {
        return None;
    }
    let mut items = Vec::with_capacity(cached.items.len() + read.items.len());
    items.extend(cached.items.iter().cloned());
    items.extend(read.items);
    Some(Parsed {
        inode: file.inode,
        size: file.size,
        modified: file.modified,
        items: Arc::new(items),
        resume: Some(Resume {
            offset: read.offset,
            fingerprint: fingerprint(&file.path, read.offset)?,
            parser: read.parser,
        }),
    })
}

/// What one kind of log parsed to, by file, in memory and optionally on
/// disk under `dir`: a `manifest.json` naming each file's record in
/// `records/`. A record is written only when its file was parsed again.
pub struct ScanCache<P: Parser> {
    dir: Option<PathBuf>,
    /// Bumped when what the parser yields changes, so old records are
    /// parsed again rather than misread.
    schema: u32,
    files: Mutex<Option<HashMap<PathBuf, Parsed<P>>>>,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: u32,
    schema: u32,
    /// Each file's path, and the name of its record.
    files: Vec<(PathBuf, String)>,
}

impl<P: Parser> ScanCache<P> {
    pub const fn new(dir: Option<PathBuf>, schema: u32) -> Self {
        Self {
            dir,
            schema,
            files: Mutex::new(None),
        }
    }

    /// What each of `files` modified at or after `since` (nanoseconds since
    /// the epoch) holds, by path: parsed afresh where it changed, from the
    /// cache where it didn't. A file that can't be read is left out and not
    /// cached. Files not listed are forgotten, so each call lists them all.
    pub fn parsed(&self, files: &[LogFile], since: i128) -> HashMap<PathBuf, Arc<Vec<P::Item>>> {
        let in_window: Vec<&LogFile> = files.iter().filter(|file| file.modified >= since).collect();
        let cached = {
            let mut guard = self.files.lock().unwrap_or_else(PoisonError::into_inner);
            guard.get_or_insert_with(|| self.load()).clone()
        };
        let stale: Vec<&LogFile> = in_window
            .iter()
            .copied()
            .filter(|file| {
                !cached
                    .get(&file.path)
                    .is_some_and(|parsed| parsed.is_of(file))
            })
            .collect();
        let mut fresh = parse_all(&stale, &cached);
        let mut next: HashMap<PathBuf, Parsed<P>> = HashMap::with_capacity(in_window.len());
        let mut written = Vec::new();
        for file in in_window {
            let entry = match fresh.remove(&file.path) {
                Some(Some(parsed)) => {
                    written.push(file.path.clone());
                    Some(parsed)
                }
                Some(None) => None,
                None => cached.get(&file.path).cloned(),
            };
            if let Some(entry) = entry {
                next.insert(file.path.clone(), entry);
            }
        }
        if !written.is_empty() || next.len() != cached.len() {
            self.save(&next, &written);
        }
        let items = next
            .iter()
            .map(|(path, parsed)| (path.clone(), Arc::clone(&parsed.items)))
            .collect();
        *self.files.lock().unwrap_or_else(PoisonError::into_inner) = Some(next);
        items
    }

    fn load(&self) -> HashMap<PathBuf, Parsed<P>> {
        let Some(dir) = &self.dir else {
            return HashMap::new();
        };
        let Some(manifest) = fs::read(dir.join("manifest.json"))
            .ok()
            .and_then(|body| serde_json::from_slice::<Manifest>(&body).ok())
            .filter(|manifest| manifest.format == FORMAT_VERSION && manifest.schema == self.schema)
        else {
            return HashMap::new();
        };
        manifest
            .files
            .into_iter()
            .filter_map(|(path, record)| {
                let body = fs::read(dir.join("records").join(record)).ok()?;
                Some((path, serde_json::from_slice(&body).ok()?))
            })
            .collect()
    }

    /// Writes the records of `written`, drops those of files gone, and
    /// rewrites the manifest. A failure only costs a re-read next launch.
    fn save(&self, files: &HashMap<PathBuf, Parsed<P>>, written: &[PathBuf]) {
        let Some(dir) = &self.dir else {
            return;
        };
        let records = dir.join("records");
        let result = (|| -> Result<(), String> {
            fs::create_dir_all(&records).map_err(|err| err.to_string())?;
            for path in written {
                if let Some(parsed) = files.get(path) {
                    let body = serde_json::to_vec(parsed).map_err(|err| err.to_string())?;
                    atomic_write(&records.join(record_name(path)), &body)
                        .map_err(|err| err.message())?;
                }
            }
            let mut listed: Vec<(PathBuf, String)> = files
                .keys()
                .map(|path| (path.clone(), record_name(path)))
                .collect();
            listed.sort();
            let manifest = Manifest {
                format: FORMAT_VERSION,
                schema: self.schema,
                files: listed,
            };
            let body = serde_json::to_vec(&manifest).map_err(|err| err.to_string())?;
            atomic_write(&dir.join("manifest.json"), &body).map_err(|err| err.message())?;
            let kept: std::collections::HashSet<String> =
                files.keys().map(|path| record_name(path)).collect();
            for entry in fs::read_dir(&records).into_iter().flatten().flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !kept.contains(&name) {
                    let _ = fs::remove_file(entry.path());
                }
            }
            Ok(())
        })();
        if let Err(err) = result {
            eprintln!("claude-switch: could not save what the session logs held: {err}");
        }
    }
}

/// A record's file name: a hash of the log's path.
fn record_name(path: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.as_os_str().as_encoded_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}.json")
}

/// Parses `files`, several at once, each on from its `cached` entry where
/// it can be. `None` for a file that couldn't be read.
fn parse_all<P: Parser>(
    files: &[&LogFile],
    cached: &HashMap<PathBuf, Parsed<P>>,
) -> HashMap<PathBuf, Option<Parsed<P>>> {
    let next = AtomicUsize::new(0);
    let results = Mutex::new(HashMap::with_capacity(files.len()));
    std::thread::scope(|scope| {
        for _ in 0..PARSE_THREADS.min(files.len()) {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(file) = files.get(index) else {
                    break;
                };
                let parsed = parse(file, cached.get(&file.path));
                results
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(file.path.clone(), parsed);
            });
        }
    });
    results.into_inner().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// Yields each line as text, and counts the lines it has seen, so a
    /// resumed read shows it kept its state.
    #[derive(Clone, Default, Serialize, Deserialize)]
    struct Lines {
        seen: usize,
    }

    impl Parser for Lines {
        type Item = (usize, String);

        fn line(&mut self, line: &[u8], items: &mut Vec<Self::Item>) {
            self.seen += 1;
            items.push((self.seen, String::from_utf8_lossy(line).into_owned()));
        }
    }

    fn append(path: &Path, text: &str) {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    fn texts(items: &[(usize, String)]) -> Vec<String> {
        items
            .iter()
            .map(|(seen, text)| format!("{seen}:{text}"))
            .collect()
    }

    /// The items of `files`, in their order, as the cache has them.
    fn items(cache: &ScanCache<Lines>, files: &[LogFile], since: i128) -> Vec<(usize, String)> {
        let parsed = cache.parsed(files, since);
        files
            .iter()
            .filter_map(|file| parsed.get(&file.path))
            .flat_map(|items| items.iter().cloned())
            .collect()
    }

    fn scan(cache: &ScanCache<Lines>, path: &Path) -> Vec<String> {
        texts(&items(cache, &[LogFile::at(path).unwrap()], 0))
    }

    #[test]
    fn a_file_that_grew_is_read_on_with_its_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        append(&path, "one\ntwo\n");
        let cache = ScanCache::<Lines>::new(None, 1);
        assert_eq!(scan(&cache, &path), ["1:one", "2:two"]);
        append(&path, "three\n");
        // A fresh read would number "three" 1 if it hadn't resumed; it can
        // only be 3 by carrying on from the saved state.
        assert_eq!(scan(&cache, &path), ["1:one", "2:two", "3:three"]);
    }

    #[test]
    fn a_rewritten_file_is_read_afresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        append(&path, "one\ntwo\n");
        let cache = ScanCache::<Lines>::new(None, 1);
        scan(&cache, &path);
        let inode = LogFile::at(&path).unwrap().inode;
        let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.write_all(b"ONE\ntwo\nthree\n").unwrap();
        drop(file);
        assert_eq!(LogFile::at(&path).unwrap().inode, inode);
        assert_eq!(scan(&cache, &path), ["1:ONE", "2:two", "3:three"]);
    }

    #[test]
    fn a_line_still_being_written_is_read_but_not_resumed_from() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        append(&path, "one\ntw");
        let cache = ScanCache::<Lines>::new(None, 1);
        assert_eq!(scan(&cache, &path), ["1:one", "2:tw"]);
        append(&path, "o\n");
        assert_eq!(scan(&cache, &path), ["1:one", "2:two"]);
    }

    #[test]
    fn a_record_over_the_limit_is_skipped_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        let huge = "x".repeat(MAX_RECORD_BYTES + 10);
        append(&path, &format!("one\n{huge}\ntwo\n\n"));
        let cache = ScanCache::<Lines>::new(None, 1);
        assert_eq!(scan(&cache, &path), ["1:one", "2:two"]);
    }

    #[test]
    fn files_out_of_the_window_or_gone_are_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.jsonl"), dir.path().join("b.jsonl"));
        append(&a, "a\n");
        append(&b, "b\n");
        let files = jsonl_files(dir.path());
        assert_eq!(files.len(), 2);
        let cache = ScanCache::<Lines>::new(None, 1);
        let newest = files.iter().map(|file| file.modified).max().unwrap();
        assert!(items(&cache, &files, newest + 1).is_empty());
        assert_eq!(texts(&items(&cache, &files, 0)), ["1:a", "1:b"]);
        fs::remove_file(&b).unwrap();
        let files = jsonl_files(dir.path());
        assert_eq!(texts(&items(&cache, &files, 0)), ["1:a"]);
    }

    #[test]
    fn the_cache_on_disk_survives_a_relaunch_and_a_schema_change_drops_it() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let path = logs.join("a.jsonl");
        append(&path, "one\n");
        let store = dir.path().join("cache");
        scan(&ScanCache::<Lines>::new(Some(store.clone()), 1), &path);
        append(&path, "two\n");
        // A new cache reads the record and goes on from it.
        let relaunched = ScanCache::<Lines>::new(Some(store.clone()), 1);
        assert_eq!(scan(&relaunched, &path), ["1:one", "2:two"]);
        assert_eq!(fs::read_dir(store.join("records")).unwrap().count(), 1);
        // A different schema reads the file afresh.
        let changed = ScanCache::<Lines>::new(Some(store), 2);
        assert_eq!(scan(&changed, &path), ["1:one", "2:two"]);
    }

    #[test]
    fn a_compressed_log_reads_like_a_plain_one() {
        let dir = tempfile::tempdir().unwrap();
        // "one\ntwo\n" compressed by `zstd`.
        let compressed: &[u8] = &[
            0x28, 0xb5, 0x2f, 0xfd, 0x04, 0x58, 0x41, 0x00, 0x00, 0x6f, 0x6e, 0x65, 0x0a, 0x74,
            0x77, 0x6f, 0x0a, 0xe6, 0xa6, 0x0e, 0x40,
        ];
        let path = dir.path().join("rollout.jsonl.zst");
        fs::write(&path, compressed).unwrap();
        let files = jsonl_files(dir.path());
        assert_eq!(files.len(), 1);
        let cache = ScanCache::<Lines>::new(None, 1);
        assert_eq!(texts(&items(&cache, &files, 0)), ["1:one", "2:two"]);
        fs::write(&path, b"not zstd").unwrap();
        let files = jsonl_files(dir.path());
        assert!(items(&cache, &files, 0).is_empty());
    }

    #[test]
    fn links_are_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        append(&real.join("a.jsonl"), "a\n");
        let scanned = dir.path().join("scanned");
        fs::create_dir(&scanned).unwrap();
        std::os::unix::fs::symlink(&real, scanned.join("linked")).unwrap();
        std::os::unix::fs::symlink(real.join("a.jsonl"), scanned.join("b.jsonl")).unwrap();
        assert!(jsonl_files(&scanned).is_empty());
        // The directory scanned may itself be a link.
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        assert_eq!(jsonl_files(&alias).len(), 1);
    }
}
