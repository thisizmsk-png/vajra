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

/// Read the campaign `slug` under `campaigns_dir`. `seam(stage)` is called after the dir is open (`"dir"`) and
/// before each file is opened (`"file:<name>"`); production passes a no-op, tests use it to swap files mid-read.
pub fn read_campaign(campaigns_dir: &Path, slug: &str, seam: &dyn Fn(&str)) -> Result<Held, HeldError> {
    read_campaign_as(campaigns_dir, slug, unsafe { libc::getuid() }, seam)
}

/// `read_campaign` with the user id files must belong to (tests pass another one).
fn read_campaign_as(campaigns_dir: &Path, slug: &str, uid: u32, seam: &dyn Fn(&str)) -> Result<Held, HeldError> {
    if slug.is_empty() || slug.contains('/') || slug == "." || slug == ".." {
        return Err(HeldError::Failed("bad slug".into()));
    }
    let root = open_dir(None, campaigns_dir.to_str().ok_or_else(|| HeldError::Failed("path".into()))?, uid)?;
    let dir = open_dir(Some(root.as_raw_fd()), slug, uid)?;
    seam("dir");
    let fd = dir.as_raw_fd();
    let campaign = read_file(fd, "campaign.json", CAP_CAMPAIGN, true, uid, seam)?.unwrap_or_default();
    let features = read_file(fd, "features.json", CAP_FEATURES, true, uid, seam)?.unwrap_or_default();
    let progress = read_file(fd, "progress.md", CAP_PROGRESS, true, uid, seam)?.unwrap_or_default();
    let sig = read_file(fd, "state.sig", CAP_SIG, true, uid, seam)?.unwrap_or_default();
    let report = read_file(fd, "report.md", CAP_REPORT, false, uid, seam)?;
    Ok(Held { campaign, features, progress, sig, report })
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

fn read_file(dir: RawFd, name: &'static str, cap: usize, required: bool, uid: u32, seam: &dyn Fn(&str)) -> Result<Option<Vec<u8>>, HeldError> {
    seam(&format!("file:{name}"));
    let c = CString::new(name).unwrap();
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
    fn read(d: &Path) -> Result<Held, HeldError> {
        read_campaign(d, "camp", &nop)
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
        assert_eq!(read_campaign(&d, "nope", &nop).unwrap_err(), HeldError::NotFound);
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
        assert!(matches!(read_campaign(&d, "alias", &nop), Err(HeldError::Failed(_)) | Err(HeldError::NotFound)));
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
        assert!(matches!(read_campaign(&d, "camp", &seam), Err(HeldError::Failed(_))));
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
        assert!(matches!(read_campaign_as(&d, "camp", other, &nop), Err(HeldError::Failed(_))));
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
        let key = format!(".hmac{}", "-key");
        fs::write(d.join("camp").join(&key), b"secret").unwrap();
        let seen = std::cell::RefCell::new(Vec::new());
        let seam = |st: &str| seen.borrow_mut().push(st.to_string());
        read_campaign(&d, "camp", &seam).unwrap();
        let seen = seen.into_inner();
        assert_eq!(seen, ["dir", "file:campaign.json", "file:features.json", "file:progress.md", "file:state.sig", "file:report.md"]);
        assert!(seen.iter().all(|s| !s.contains(&key)));
    }
}
