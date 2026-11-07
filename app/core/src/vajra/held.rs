//! Held bytes: read a campaign's signed inputs exactly once and keep the bytes.
//!
//! Everything the report path trusts (the signature check, `featuresDefSha256`, `reportSha256`, the text shown) is
//! derived from the `Vec<u8>`s returned here, never from a second read of any file, so a same-user process that
//! rewrites the campaign dir while a check runs can change nothing that was already read. The dir and every file are
//! opened without following links, must be regular files owned by this user, and are read through a size cap
//! (`take(cap + 1)`), so a growing file or a FIFO cannot hang or exhaust memory. Design: vajra-13
//! `integrity-verify-bytes.md`.

use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;

pub const CAP_CAMPAIGN: usize = 256 * 1024;
pub const CAP_FEATURES: usize = 1024 * 1024;
pub const CAP_PROGRESS: usize = 4 * 1024 * 1024;
pub const CAP_SIG: usize = 256;
pub const CAP_REPORT: usize = 64 * 1024;

/// The five inputs, exactly as they were on disk when read.
#[derive(Debug)]
pub struct Held {
    pub campaign: Vec<u8>,
    pub features: Vec<u8>,
    pub progress: Vec<u8>,
    pub sig: Vec<u8>,
    /// `None` when the campaign has no `report.md` (a missing file is not an error; a link or a FIFO is).
    pub report: Option<Vec<u8>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HeldError {
    /// The campaign dir does not exist.
    NotFound,
    /// A link, a non-regular or foreign-owned file, or a missing required file: the integrity check cannot pass.
    Failed(String),
    /// A file is larger than its cap: the verdict is unknown, never ok.
    TooLarge(&'static str),
    Io(String),
}

/// Read the campaign `slug` under `campaigns_dir`. `key` is the signing key's path: it is only `stat`ed (never
/// opened) so that no input can be the key's inode (NFR-SEC-2). `seam(stage)` is called after the dir is open
/// (`"dir"`), before each file is looked at (`"file:<name>"`) and between that look and the open (`"open:<name>"`);
/// production passes a no-op, tests use it to swap files mid-read.
pub fn read_campaign(campaigns_dir: &Path, slug: &str, key: &Path, seam: &dyn Fn(&str)) -> Result<Held, HeldError> {
    read_campaign_as(campaigns_dir, slug, key, unsafe { libc::getuid() }, seam)
}

/// The key's identity, `(st_dev, st_ino)`, for the path itself and for what it points at if it is a link. Metadata
/// only: the key is never opened or read.
fn key_ids(key: &Path) -> Vec<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let mut ids: Vec<(u64, u64)> =
        [std::fs::symlink_metadata(key), std::fs::metadata(key)].into_iter().flatten().map(|m| (m.dev(), m.ino())).collect();
    ids.dedup();
    ids
}

/// `read_campaign` with the user id files must belong to (tests pass another one).
fn read_campaign_as(campaigns_dir: &Path, slug: &str, key: &Path, uid: u32, seam: &dyn Fn(&str)) -> Result<Held, HeldError> {
    if slug.is_empty() || slug.contains('/') || slug == "." || slug == ".." {
        return Err(HeldError::Failed("bad slug".into()));
    }
    let root = open_dir(None, campaigns_dir.to_str().ok_or_else(|| HeldError::Failed("path".into()))?, uid)?;
    let dir = open_dir(Some(root.as_raw_fd()), slug, uid)?;
    seam("dir");
    let keys = key_ids(key);
    let rd = |name: &'static str, cap: usize, required: bool| read_file(dir.as_raw_fd(), name, cap, required, uid, &keys, seam);
    let campaign = rd("campaign.json", CAP_CAMPAIGN, true)?.unwrap_or_default();
    let features = rd("features.json", CAP_FEATURES, true)?.unwrap_or_default();
    let progress = rd("progress.md", CAP_PROGRESS, true)?.unwrap_or_default();
    let sig = rd("state.sig", CAP_SIG, true)?.unwrap_or_default();
    let report = rd("report.md", CAP_REPORT, false)?;
    Ok(Held { campaign, features, progress, sig, report })
}

/// A held input must be a plain single-link file that is not the signing key. `O_NOFOLLOW` stops symlinks only; a
/// hard link to the key (or to anything else) placed under an allowed name has that name but another file's data.
fn single_link_not_key(st: &libc::stat, name: &str, keys: &[(u64, u64)]) -> Result<(), HeldError> {
    if st.st_nlink != 1 {
        return Err(HeldError::Failed(format!("{name}: has more than one link")));
    }
    if keys.contains(&(st.st_dev as u64, st.st_ino)) {
        return Err(HeldError::Failed(format!("{name}: is the signing key")));
    }
    Ok(())
}

fn open_dir(parent: Option<RawFd>, name: &str, uid: u32) -> Result<OwnedFd, HeldError> {
    let c = CString::new(name).map_err(|_| HeldError::Failed("nul in path".into()))?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe {
        match parent {
            Some(p) => libc::openat(p, c.as_ptr(), flags),
            None => libc::open(c.as_ptr(), flags),
        }
    };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        return Err(match e.raw_os_error() {
            Some(libc::ENOENT) => HeldError::NotFound,
            Some(libc::ELOOP) | Some(libc::ENOTDIR) => HeldError::Failed(format!("{name}: not a plain directory")),
            _ => HeldError::Io(e.to_string()),
        });
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    check_owner(fd.as_raw_fd(), true, name, uid)?;
    Ok(fd)
}

fn check_owner(fd: RawFd, want_dir: bool, name: &str, uid: u32) -> Result<libc::stat, HeldError> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } != 0 {
        return Err(HeldError::Io(std::io::Error::last_os_error().to_string()));
    }
    let kind = st.st_mode & libc::S_IFMT;
    if kind != if want_dir { libc::S_IFDIR } else { libc::S_IFREG } {
        return Err(HeldError::Failed(format!("{name}: wrong file type")));
    }
    if st.st_uid != uid {
        return Err(HeldError::Failed(format!("{name}: not owned by this user")));
    }
    Ok(st)
}

fn read_file(
    dir: RawFd,
    name: &'static str,
    cap: usize,
    required: bool,
    uid: u32,
    keys: &[(u64, u64)],
    seam: &dyn Fn(&str),
) -> Result<Option<Vec<u8>>, HeldError> {
    seam(&format!("file:{name}"));
    let c = CString::new(name).unwrap();
    // Look before opening, so a hard link to the key is refused without the key ever being opened. A swap after
    // this look is caught by the same check on the open descriptor below, still before any byte is read.
    let mut pre: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatat(dir, c.as_ptr(), &mut pre, libc::AT_SYMLINK_NOFOLLOW) } == 0
        && pre.st_mode & libc::S_IFMT == libc::S_IFREG
    {
        single_link_not_key(&pre, name, keys)?;
    }
    seam(&format!("open:{name}"));
    let fd = unsafe { libc::openat(dir, c.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        return match e.raw_os_error() {
            Some(libc::ENOENT) if !required => Ok(None),
            Some(libc::ENOENT) => Err(HeldError::Failed(format!("{name}: missing"))),
            Some(libc::ELOOP) => Err(HeldError::Failed(format!("{name}: is a link"))),
            _ => Err(HeldError::Io(format!("{name}: {e}"))),
        };
    }
    let mut f = unsafe { File::from_raw_fd(fd) };
    let st = check_owner(f.as_raw_fd(), false, name, uid)?;
    single_link_not_key(&st, name, keys)?;
    if st.st_size as u64 > cap as u64 {
        return Err(HeldError::TooLarge(name));
    }
    let mut buf = Vec::new();
    f.by_ref().take(cap as u64 + 1).read_to_end(&mut buf).map_err(|e| HeldError::Io(format!("{name}: {e}")))?;
    if buf.len() > cap {
        return Err(HeldError::TooLarge(name));
    }
    Ok(Some(buf))
}

/// True when `bytes` is one valid JSON document in which no object repeats a key (at any depth). jq and serde
/// disagree about duplicate keys, so a document that repeats one is refused outright.
pub fn json_without_duplicate_keys(bytes: &[u8]) -> bool {
    let mut de = serde_json::Deserializer::from_slice(bytes);
    serde::de::DeserializeSeed::deserialize(NoDup, &mut de).is_ok() && de.end().is_ok()
}

struct NoDup;
impl<'de> serde::de::DeserializeSeed<'de> for NoDup {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(NoDup)
    }
}
impl<'de> serde::de::Visitor<'de> for NoDup {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("json")
    }
    fn visit_bool<E>(self, _: bool) -> Result<(), E> { Ok(()) }
    fn visit_i64<E>(self, _: i64) -> Result<(), E> { Ok(()) }
    fn visit_u64<E>(self, _: u64) -> Result<(), E> { Ok(()) }
    fn visit_f64<E>(self, _: f64) -> Result<(), E> { Ok(()) }
    fn visit_str<E>(self, _: &str) -> Result<(), E> { Ok(()) }
    fn visit_unit<E>(self) -> Result<(), E> { Ok(()) }
    fn visit_none<E>(self) -> Result<(), E> { Ok(()) }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
        while a.next_element_seed(NoDup)?.is_some() {}
        Ok(())
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
        let mut seen = std::collections::HashSet::new();
        while let Some(k) = a.next_key::<String>()? {
            if !seen.insert(k) {
                return Err(serde::de::Error::custom("duplicate key"));
            }
            a.next_value_seed(NoDup)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn dir() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!("vajra-held-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("camp")).unwrap();
        for (n, b) in [("campaign.json", "{}"), ("features.json", "{\"features\":[]}"), ("progress.md", "p"), ("state.sig", "ab")] {
            fs::write(d.join("camp").join(n), b).unwrap();
        }
        d
    }
    fn nop(_: &str) {}
    /// A dummy stand-in for the signing key, inside the test's temp dir (never the real one).
    fn key(d: &Path) -> PathBuf {
        d.join("dummy-signing-key")
    }
    fn read(d: &Path) -> Result<Held, HeldError> {
        read_campaign(d, "camp", &key(d), &nop)
    }

    #[test]
    fn reads_the_inputs_once_and_keeps_the_exact_bytes() {
        let d = dir();
        fs::write(d.join("camp/report.md"), b"a\0b\n").unwrap(); // a NUL survives: bytes, not a shell variable
        let h = read(&d).unwrap();
        assert_eq!(h.campaign, b"{}");
        assert_eq!(h.sig, b"ab");
        assert_eq!(h.report.as_deref(), Some(&b"a\0b\n"[..]));
    }

    #[test]
    fn a_missing_report_is_none_but_a_missing_required_file_fails() {
        let d = dir();
        assert!(read(&d).unwrap().report.is_none());
        fs::remove_file(d.join("camp/progress.md")).unwrap();
        assert!(matches!(read(&d), Err(HeldError::Failed(_))));
    }

    #[test]
    fn a_missing_campaign_dir_is_not_found() {
        let d = dir();
        assert_eq!(read_campaign(&d, "nope", &key(&d), &nop).unwrap_err(), HeldError::NotFound);
    }

    #[test]
    fn each_file_at_exactly_its_cap_is_accepted_and_one_byte_more_is_too_large() {
        for (name, cap) in [("campaign.json", CAP_CAMPAIGN), ("features.json", CAP_FEATURES), ("progress.md", CAP_PROGRESS), ("state.sig", CAP_SIG), ("report.md", CAP_REPORT)] {
            let d = dir();
            fs::write(d.join("camp").join(name), vec![b'x'; cap]).unwrap();
            assert!(read(&d).is_ok(), "{name} at cap");
            fs::write(d.join("camp").join(name), vec![b'x'; cap + 1]).unwrap();
            assert!(matches!(read(&d), Err(HeldError::TooLarge(n)) if n == name), "{name} over cap");
        }
    }

    #[test]
    fn a_file_far_over_its_cap_is_cut_off_not_read_whole() {
        let d = dir();
        fs::write(d.join("camp/campaign.json"), vec![b'x'; 20 * 1024 * 1024]).unwrap();
        assert!(matches!(read(&d), Err(HeldError::TooLarge(_))));
    }

    #[test]
    fn a_symlinked_input_fails() {
        let d = dir();
        fs::write(d.join("real-report"), b"r").unwrap();
        symlink(d.join("real-report"), d.join("camp/report.md")).unwrap();
        assert!(matches!(read(&d), Err(HeldError::Failed(_))));
        let d = dir();
        fs::rename(d.join("camp/campaign.json"), d.join("elsewhere")).unwrap();
        symlink(d.join("elsewhere"), d.join("camp/campaign.json")).unwrap();
        assert!(matches!(read(&d), Err(HeldError::Failed(_))));
    }

    #[test]
    fn a_symlinked_campaign_dir_is_refused() {
        let d = dir();
        symlink(d.join("camp"), d.join("alias")).unwrap();
        assert!(matches!(read_campaign(&d, "alias", &key(&d), &nop), Err(HeldError::Failed(_)) | Err(HeldError::NotFound)));
    }

    #[test]
    fn a_fifo_is_refused_without_hanging() {
        let d = dir();
        let p = CString::new(d.join("camp/report.md").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(p.as_ptr(), 0o600) }, 0);
        assert!(matches!(read(&d), Err(HeldError::Failed(_))));
    }

    #[test]
    fn a_file_swapped_for_a_symlink_after_the_dir_was_opened_fails() {
        let d = dir();
        fs::write(d.join("camp/report.md"), b"genuine").unwrap();
        fs::write(d.join("elsewhere"), b"planted").unwrap();
        let root = d.clone();
        let seam = move |stage: &str| {
            if stage == "file:report.md" {
                fs::remove_file(root.join("camp/report.md")).unwrap();
                symlink(root.join("elsewhere"), root.join("camp/report.md")).unwrap();
            }
        };
        assert!(matches!(read_campaign(&d, "camp", &key(&d), &seam), Err(HeldError::Failed(_))));
    }

    #[test]
    fn rewriting_files_after_the_read_changes_nothing_that_was_read() {
        let d = dir();
        let h = read(&d).unwrap();
        for n in ["campaign.json", "features.json", "progress.md", "state.sig"] {
            fs::write(d.join("camp").join(n), b"rewritten").unwrap();
        }
        assert_eq!(h.campaign, b"{}");
        assert_eq!(h.progress, b"p");
    }

    #[test]
    fn duplicate_keys_are_refused_at_any_depth_and_trailing_garbage_too() {
        assert!(json_without_duplicate_keys(br#"{"a":1,"b":{"c":[1,{"d":2}]}}"#));
        assert!(!json_without_duplicate_keys(br#"{"a":1,"a":2}"#));
        assert!(!json_without_duplicate_keys(br#"{"x":[{"k":1,"k":1}]}"#));
        assert!(!json_without_duplicate_keys(br#"{"a":1} x"#));
        assert!(!json_without_duplicate_keys(b"not json"));
        assert!(!json_without_duplicate_keys(b""));
    }

    #[test]
    fn a_file_owned_by_someone_else_is_refused() {
        let d = dir();
        let other = unsafe { libc::getuid() } + 1;
        assert!(matches!(read_campaign_as(&d, "camp", &key(&d), other, &nop), Err(HeldError::Failed(_))));
    }

    #[test]
    fn a_file_growing_during_the_read_is_bounded_by_the_cap() {
        let d = dir();
        let report = d.join("camp/report.md");
        fs::write(&report, vec![b'x'; CAP_REPORT - 10]).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (s2, r2) = (stop.clone(), report.clone());
        let t = std::thread::spawn(move || {
            use std::io::Write;
            while !s2.load(Ordering::SeqCst) {
                if let Ok(mut f) = fs::OpenOptions::new().append(true).open(&r2) {
                    let _ = f.write_all(&[b'y'; 4096]);
                }
            }
        });
        for _ in 0..50 {
            match read(&d) {
                Ok(h) => assert!(h.report.unwrap().len() <= CAP_REPORT),
                Err(HeldError::TooLarge("report.md")) => {}
                Err(e) => panic!("{e:?}"),
            }
        }
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
    }

    #[test]
    fn the_reader_opens_only_the_five_named_files_never_the_signing_key() {
        let d = dir();
        let key_name = format!(".hmac{}", "-key");
        fs::write(d.join("camp").join(&key_name), b"secret").unwrap();
        let seen = std::cell::RefCell::new(Vec::new());
        let seam = |st: &str| seen.borrow_mut().push(st.to_string());
        read_campaign(&d, "camp", &key(&d), &seam).unwrap();
        let seen = seen.into_inner();
        let want: Vec<String> = std::iter::once("dir".to_string())
            .chain(["campaign.json", "features.json", "progress.md", "state.sig", "report.md"].iter().flat_map(|n| {
                // report.md is optional and absent here: it is looked at, then openat reports ENOENT.
                [format!("file:{n}"), format!("open:{n}")]
            }))
            .collect();
        assert_eq!(seen, want);
        assert!(seen.iter().all(|s| !s.contains(&key_name)));
    }

    // ---- review #34 (P1): hard links. O_NOFOLLOW stops symlinks only. -------------------------------------------

    const CANARY: &[u8] = b"DUMMY-KEY-CANARY-0f3a"; // a test value in a temp dir, never a real key

    #[test]
    fn a_hard_link_to_the_signing_key_is_refused_for_every_input() {
        for name in ["campaign.json", "features.json", "progress.md", "state.sig", "report.md"] {
            let d = dir();
            fs::write(key(&d), CANARY).unwrap();
            let p = d.join("camp").join(name);
            let _ = fs::remove_file(&p);
            fs::hard_link(key(&d), &p).unwrap();
            let r = read(&d);
            match &r {
                Err(HeldError::Failed(why)) => assert!(!why.as_bytes().windows(CANARY.len()).any(|w| w == CANARY), "{why}"),
                other => panic!("{name}: a hard link to the key must fail, got {other:?}"),
            }
        }
    }

    #[test]
    fn any_input_with_more_than_one_link_is_refused() {
        // A hard link to an unrelated file of the same user: nlink 2, so it is not a plain single-link input.
        let d = dir();
        let elsewhere = d.join("elsewhere.md");
        fs::write(&elsewhere, b"p").unwrap();
        fs::remove_file(d.join("camp/progress.md")).unwrap();
        fs::hard_link(&elsewhere, d.join("camp/progress.md")).unwrap();
        assert!(matches!(read(&d), Err(HeldError::Failed(w)) if w.contains("more than one link")));
    }

    #[test]
    fn an_input_that_is_the_keys_inode_is_refused_even_with_a_single_link() {
        // The inode check stands on its own: here the key path is a symlink to the input, so the input has one link
        // but the same (st_dev, st_ino) as what the key path resolves to.
        let d = dir();
        symlink(d.join("camp/state.sig"), key(&d)).unwrap();
        assert!(matches!(read(&d), Err(HeldError::Failed(w)) if w.contains("is the signing key")));
    }

    #[test]
    fn a_hard_link_swapped_in_after_the_pre_open_look_is_caught_on_the_descriptor() {
        // The look before openat sees a plain file; the seam swaps in a hard link to the key right after it, so only
        // the fstat on the open descriptor can catch it, and it must, before any byte is read.
        let d = dir();
        fs::write(key(&d), CANARY).unwrap();
        let target = d.join("camp/progress.md");
        let k = key(&d);
        let seam = |st: &str| {
            if st == "open:progress.md" {
                fs::remove_file(&target).unwrap();
                fs::hard_link(&k, &target).unwrap();
            }
        };
        let r = read_campaign(&d, "camp", &key(&d), &seam);
        assert!(matches!(&r, Err(HeldError::Failed(w)) if w.starts_with("progress.md")), "{r:?}");
    }

    #[test]
    fn a_missing_key_file_changes_nothing() {
        let d = dir(); // no dummy key at all
        assert!(read(&d).is_ok());
    }
}
