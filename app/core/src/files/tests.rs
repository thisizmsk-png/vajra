use super::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

/// `<tmp>/root` (the project), `<tmp>/outside`, `<tmp>/state/<key>` (a dummy key, never a real one) and
/// `<tmp>/root/appdata` (a denied folder that happens to sit inside the root).
struct Fx {
    dir: PathBuf,
}

const CANARY: &[u8] = b"DUMMY-KEY-CANARY-77c1";

impl Fx {
    fn new() -> Self {
        let d = std::env::temp_dir().join(format!("vajra-files-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        for sub in ["root/src", "root/.git", "root/appdata", "outside", "state"] {
            fs::create_dir_all(d.join(sub)).unwrap();
        }
        let dir = d.canonicalize().unwrap();
        fs::write(dir.join("root/src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.join("root/README.md"), "# hi\n").unwrap();
        fs::write(dir.join("root/.git/config"), "[core]\n").unwrap();
        fs::write(dir.join("root/appdata/secret.json"), "{}").unwrap();
        fs::write(dir.join("outside/x.txt"), "outside").unwrap();
        fs::write(dir.join("state").join(DENIED_NAME), CANARY).unwrap();
        Self { dir }
    }
    fn root(&self) -> PathBuf {
        self.dir.join("root")
    }
    fn key(&self) -> PathBuf {
        self.dir.join("state").join(DENIED_NAME)
    }
    fn scope(&self) -> Scope {
        Scope::new(&self.root(), &[self.root().join("appdata"), self.dir.join("state")], &self.key()).unwrap()
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn text(r: FileRead) -> String {
    match r.content {
        Content::Text(s) => s,
        other => panic!("not text: {other:?}"),
    }
}

fn never_the_key(r: &Result<FileRead, Refused>) {
    if let Ok(FileRead { content: Content::Text(s), .. }) = r {
        assert!(!s.as_bytes().windows(CANARY.len()).any(|w| w == CANARY), "the key's bytes were read");
    }
}

#[test]
fn a_file_and_the_root_listing_inside_the_project_are_served() {
    let fx = Fx::new();
    let s = fx.scope();
    assert_eq!(text(s.read("src/main.rs").unwrap()), "fn main() {}\n");
    let names: Vec<_> = s.list("").unwrap().entries.into_iter().map(|e| (e.name, e.kind)).collect();
    // Folders first; `.git` and the denied folder are not listed.
    assert_eq!(names, vec![("src".to_string(), "dir"), ("README.md".to_string(), "file")]);
}

#[test]
fn absolute_paths_are_refused() {
    let s = Fx::new().scope();
    for p in ["/etc/passwd", "/", "//etc/hosts"] {
        assert_eq!(s.read(p).unwrap_err().code(), "invalid_args", "{p}");
        assert_eq!(s.list(p).unwrap_err().code(), "invalid_args", "{p}");
    }
    let fx = Fx::new();
    let abs = fx.root().join("README.md");
    assert_eq!(fx.scope().read(abs.to_str().unwrap()).unwrap_err().code(), "invalid_args", "even one inside the root");
}

#[test]
fn dot_dot_dot_and_empty_parts_are_refused_before_anything_is_opened() {
    let s = Fx::new().scope();
    for p in ["..", "../outside/x.txt", "src/../../outside/x.txt", "src/..", "./README.md", "src/./main.rs", "src//main.rs", "src/", "a\0b"] {
        assert_eq!(s.read(p).unwrap_err().code(), "invalid_args", "{p:?}");
    }
    assert_eq!(s.read(&"a/".repeat(PATH_MAX_LEN)).unwrap_err().code(), "invalid_args", "too long");
}

#[test]
fn git_folders_and_the_key_name_are_refused_anywhere() {
    let fx = Fx::new();
    fs::create_dir_all(fx.root().join("sub/.git")).unwrap();
    fs::write(fx.root().join("sub/.git/HEAD"), "x").unwrap();
    fs::write(fx.root().join(DENIED_NAME), "x").unwrap();
    let s = fx.scope();
    for p in [".git/config", ".git", "sub/.git/HEAD", DENIED_NAME, &format!("src/{DENIED_NAME}")] {
        assert_eq!(s.read(p).unwrap_err(), Refused::Protected, "{p}");
    }
    assert_eq!(s.list(".git").unwrap_err(), Refused::Protected);
    assert!(!s.list("").unwrap().entries.iter().any(|e| e.name == DENIED_NAME || e.name == ".git"), "not listed either");
}

#[test]
fn a_link_that_leaves_the_root_is_refused_and_listed_only_as_a_link() {
    let fx = Fx::new();
    symlink(fx.dir.join("outside/x.txt"), fx.root().join("out.txt")).unwrap();
    symlink(fx.dir.join("outside"), fx.root().join("outdir")).unwrap();
    symlink("/etc", fx.root().join("etc")).unwrap();
    let s = fx.scope();
    assert_eq!(s.read("out.txt").unwrap_err(), Refused::Outside);
    assert_eq!(s.read("outdir/x.txt").unwrap_err(), Refused::Outside);
    assert_eq!(s.list("outdir").unwrap_err(), Refused::Outside);
    assert_eq!(s.read("etc/hosts").unwrap_err(), Refused::Outside);
    // A missing file behind the link answers the same, so the link can't probe what exists outside.
    assert_eq!(s.read("outdir/missing.txt").unwrap_err(), Refused::Outside);
    assert_eq!(s.read("etc/no-such-file").unwrap_err(), Refused::Outside);
    assert_eq!(s.read("src/missing.rs").unwrap_err(), Refused::NotFound, "inside, missing is just missing");
    let l = s.list("").unwrap();
    for n in ["out.txt", "outdir", "etc"] {
        let e = l.entries.iter().find(|e| e.name == n).unwrap();
        assert_eq!((e.kind, e.link), ("link", false), "{n} is shown as a link that can't be opened");
    }
}

#[test]
fn a_link_that_stays_inside_the_root_is_followed() {
    let fx = Fx::new();
    symlink("src/main.rs", fx.root().join("main-link.rs")).unwrap();
    symlink("src", fx.root().join("src-link")).unwrap();
    let s = fx.scope();
    assert_eq!(text(s.read("main-link.rs").unwrap()), "fn main() {}\n");
    assert_eq!(text(s.read("src-link/main.rs").unwrap()), "fn main() {}\n");
    let l = s.list("").unwrap();
    let e = l.entries.iter().find(|e| e.name == "src-link").unwrap();
    assert_eq!((e.kind, e.link), ("dir", true));
}

#[test]
fn a_link_into_git_or_a_denied_folder_inside_the_root_is_refused() {
    let fx = Fx::new();
    symlink(".git/config", fx.root().join("cfg")).unwrap();
    symlink(".git", fx.root().join("g")).unwrap();
    symlink("appdata/secret.json", fx.root().join("s.json")).unwrap();
    let s = fx.scope();
    assert_eq!(s.read("cfg").unwrap_err(), Refused::Protected);
    assert_eq!(s.read("g/config").unwrap_err(), Refused::Protected);
    assert_eq!(s.list("g").unwrap_err(), Refused::Protected);
    assert_eq!(s.read("s.json").unwrap_err(), Refused::Protected);
    assert!(s.list("").unwrap().entries.iter().filter(|e| ["cfg", "g", "s.json"].contains(&e.name.as_str())).all(|e| !e.link && e.kind == "link"));
}

#[test]
fn the_app_data_folder_is_refused_even_inside_the_root() {
    let fx = Fx::new();
    let s = fx.scope();
    assert_eq!(s.read("appdata/secret.json").unwrap_err(), Refused::Protected);
    assert_eq!(s.list("appdata").unwrap_err(), Refused::Protected);
    assert!(!s.list("").unwrap().entries.iter().any(|e| e.name == "appdata"));
}

#[test]
fn a_root_inside_a_denied_folder_shows_nothing() {
    let fx = Fx::new();
    let s = Scope::new(&fx.root(), std::slice::from_ref(&fx.dir), &fx.key()).unwrap();
    assert_eq!(s.list("").unwrap_err(), Refused::Protected);
    assert_eq!(s.read("README.md").unwrap_err(), Refused::Protected);
}

#[test]
fn the_signing_key_is_refused_through_a_link_or_a_hard_link_under_another_name() {
    let fx = Fx::new();
    // A link to the key leaves the root (and the key's folder is denied).
    symlink(fx.key(), fx.root().join("k.txt")).unwrap();
    // A hard link inside the root has an innocent name and path but the key's inode.
    fs::hard_link(fx.key(), fx.root().join("notes.txt")).unwrap();
    let s = fx.scope();
    for p in ["k.txt", "notes.txt"] {
        let r = s.read(p);
        never_the_key(&r);
        assert!(r.is_err(), "{p}: {r:?}");
    }
    assert_eq!(s.read("notes.txt").unwrap_err(), Refused::Protected);
    // Even when the key's own folder is not on the deny list.
    let bare = Scope::new(&fx.root(), &[], &fx.key()).unwrap();
    assert_eq!(bare.read("notes.txt").unwrap_err(), Refused::Protected);
}

#[test]
fn a_link_swapped_in_after_the_check_is_refused_not_followed() {
    let fx = Fx::new();
    let s = fx.scope();
    let real = s.resolve("src/main.rs").unwrap();
    // Between the check and the open, `src` becomes a link to the outside folder holding a `main.rs`.
    fs::write(fx.dir.join("outside/main.rs"), "outside").unwrap();
    fs::rename(fx.root().join("src"), fx.root().join("src-old")).unwrap();
    symlink(fx.dir.join("outside"), fx.root().join("src")).unwrap();
    assert_eq!(s.open_walk(&real, false).unwrap_err(), Refused::Outside);
    // And the file itself swapped for a link.
    let real = s.resolve("README.md").unwrap();
    fs::remove_file(fx.root().join("README.md")).unwrap();
    symlink(fx.dir.join("outside/x.txt"), fx.root().join("README.md")).unwrap();
    assert_eq!(s.open_walk(&real, false).unwrap_err(), Refused::Outside);
}

#[test]
fn a_folder_above_the_root_swapped_for_a_link_after_the_check_is_refused() {
    // The root is <tmp>/root/wt; an outside folder holds a look-alike `wt` with a secret.
    let fx = Fx::new();
    let wt = fx.root().join("wt");
    fs::create_dir_all(&wt).unwrap();
    fs::write(wt.join("a.txt"), "inside").unwrap();
    fs::create_dir_all(fx.dir.join("outside/wt")).unwrap();
    fs::write(fx.dir.join("outside/wt/a.txt"), "SECRET").unwrap();
    let s = Scope::new(&wt, &[], &fx.key()).unwrap();
    let real = s.resolve("a.txt").unwrap();
    // Swap `root` (an ancestor of the scope's root) for a link to `outside`.
    fs::rename(fx.root(), fx.dir.join("root-old")).unwrap();
    symlink(fx.dir.join("outside"), fx.root()).unwrap();
    assert_eq!(s.open_walk(&real, false).unwrap_err(), Refused::Outside);
    assert!(s.read("a.txt").is_err());
    assert!(s.list("").is_err());
    // And a same-named real folder put in place is refused by identity.
    fs::remove_file(fx.root()).unwrap();
    fs::create_dir_all(fx.root().join("wt")).unwrap();
    fs::write(fx.root().join("wt/a.txt"), "SECRET").unwrap();
    assert_eq!(s.read("a.txt").unwrap_err(), Refused::Outside);
}

#[test]
fn a_large_file_is_refused_with_its_size_and_the_cap() {
    let fx = Fx::new();
    fs::write(fx.root().join("big.log"), vec![b'a'; TEXT_CAP as usize + 1]).unwrap();
    fs::write(fx.root().join("edge.log"), vec![b'a'; TEXT_CAP as usize]).unwrap();
    let s = fx.scope();
    assert_eq!(s.read("big.log").unwrap(), FileRead { size: TEXT_CAP + 1, content: Content::TooLarge { cap: TEXT_CAP } });
    assert!(matches!(s.read("edge.log").unwrap().content, Content::Text(_)), "exactly the cap is fine");
    fs::write(fx.root().join("huge.png"), vec![0u8; IMAGE_CAP as usize + 1]).unwrap();
    assert_eq!(s.read("huge.png").unwrap().content, Content::TooLarge { cap: IMAGE_CAP });
}

#[test]
fn binary_and_non_utf8_files_are_described_not_sent() {
    let fx = Fx::new();
    fs::write(fx.root().join("a.bin"), b"ELF\0\x01\x02").unwrap();
    fs::write(fx.root().join("latin.txt"), b"caf\xe9").unwrap();
    let s = fx.scope();
    assert_eq!(s.read("a.bin").unwrap().content, Content::Binary);
    assert_eq!(s.read("latin.txt").unwrap().content, Content::Binary);
}

#[test]
fn an_image_is_sent_with_its_type() {
    let fx = Fx::new();
    fs::write(fx.root().join("logo.PNG"), b"\x89PNG\r\n\x1a\n").unwrap();
    let r = fx.scope().read("logo.PNG").unwrap();
    assert_eq!(r.content, Content::Image { mime: "image/png", bytes: b"\x89PNG\r\n\x1a\n".to_vec() });
}

#[test]
fn a_fifo_a_folder_and_a_missing_file_are_refused_without_blocking() {
    let fx = Fx::new();
    let fifo = CString::new(fx.root().join("pipe").to_str().unwrap()).unwrap();
    // SAFETY: a NUL-terminated path in the test's temp dir.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let s = fx.scope();
    assert_eq!(s.read("pipe").unwrap_err(), Refused::NotAFile);
    assert_eq!(s.read("src").unwrap_err(), Refused::NotAFile);
    assert_eq!(s.read("nope.txt").unwrap_err(), Refused::NotFound);
    assert_eq!(s.list("README.md").unwrap_err(), Refused::NotADirectory);
    assert_eq!(s.list("pipe").unwrap_err(), Refused::NotADirectory);
}

#[test]
fn a_listing_is_capped() {
    let fx = Fx::new();
    let many = fx.root().join("many");
    fs::create_dir_all(&many).unwrap();
    for i in 0..LIST_CAP + 3 {
        fs::write(many.join(format!("f{i}")), "").unwrap();
    }
    let l = fx.scope().list("many").unwrap();
    assert!(l.truncated);
    assert_eq!(l.entries.len(), LIST_CAP);
}

#[test]
fn the_refusal_json_never_names_the_path_or_its_content() {
    for r in [Refused::Outside, Refused::Protected, Refused::BadPath("x"), Refused::NotFound] {
        let j = r.to_json();
        assert_eq!(j["error"], r.code());
        assert!(!j["detail"].as_str().unwrap().contains('/'));
    }
}
