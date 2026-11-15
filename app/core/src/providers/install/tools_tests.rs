use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use super::*;

/// A temp tree standing in for the machine: `sys/` is a system root, `brew/` a Homebrew prefix, `home/` the user's HOME.
struct Tree {
    dir: PathBuf,
}

impl Tree {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vajra-tools-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Tree { dir: dir.canonicalize().unwrap() }
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }
    fn file(&self, rel: &str) -> PathBuf {
        let p = self.p(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }
    fn link(&self, rel: &str, target: &Path) -> PathBuf {
        let p = self.p(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        symlink(target, &p).unwrap();
        p
    }
    /// A node + npm install under `prefix` the way Homebrew and nvm lay it out.
    fn node(&self, prefix: &str) {
        self.file(&format!("{prefix}/bin/node"));
        let cli = self.file(&format!("{prefix}/lib/node_modules/npm/bin/npm-cli.js"));
        self.link(&format!("{prefix}/bin/npm"), &cli);
    }
    fn roots(&self) -> Roots {
        Roots { system: vec![self.p("sys"), self.p("brew")], homebrew: vec![self.p("brew")], nvm: self.p("home/.nvm/versions/node"), volta: self.p("home/.volta") }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn the_production_roots_are_exactly_the_system_and_homebrew_prefixes() {
    let r = Roots::of(Path::new("/Users/sam"));
    assert_eq!(r.system, ["/usr", "/bin", "/sbin", "/opt/homebrew", "/usr/local"].map(PathBuf::from));
    assert_eq!(r.homebrew, ["/opt/homebrew", "/usr/local"].map(PathBuf::from));
    assert_eq!(r.nvm, PathBuf::from("/Users/sam/.nvm/versions/node"));
    assert_eq!(r.volta, PathBuf::from("/Users/sam/.volta"));
}

#[test]
fn brew_resolves_to_its_canonical_file_under_a_homebrew_prefix_and_is_labelled_owned_by_you() {
    let t = Tree::new("brew");
    let real = t.file("brew/Homebrew/bin/brew");
    t.link("brew/bin/brew", &real);
    let tools = resolve(&t.roots(), &[]);
    let b = tools.brew.expect("brew found");
    assert_eq!(b.path, real);
    assert!(b.owned_by_you, "a Homebrew prefix the user owns says so on the sheet");
    assert_eq!(b.label, None);
}

#[test]
fn a_brew_whose_link_leaves_the_prefix_is_never_used() {
    let t = Tree::new("brew-out");
    let evil = t.file("home/bin/brew");
    t.link("brew/bin/brew", &evil);
    assert!(resolve(&t.roots(), &[]).brew.is_none());
}

#[test]
fn npm_runs_as_node_plus_npm_cli_js_from_the_same_prefix() {
    let t = Tree::new("npm");
    t.node("brew");
    let tools = resolve(&t.roots(), &[t.p("brew/bin")]);
    let n = tools.npm.expect("npm found");
    assert_eq!(n.node.path, t.p("brew/bin/node"));
    assert_eq!(n.cli.path, t.p("brew/lib/node_modules/npm/bin/npm-cli.js"));
    assert_eq!(n.prefix, t.p("brew"));
    assert_eq!(n.argv(&["--version"]), [t.p("brew/bin/node").into_os_string(), t.p("brew/lib/node_modules/npm/bin/npm-cli.js").into_os_string(), "--version".into()]);
}

#[test]
fn a_user_dir_npm_first_on_the_path_is_never_used() {
    let t = Tree::new("npm-user");
    t.node("home/.local");
    t.node("brew");
    let tools = resolve(&t.roots(), &[t.p("home/.local/bin"), t.p("brew/bin")]);
    assert_eq!(tools.npm.expect("the Homebrew npm").prefix, t.p("brew"));
    assert!(tools.refused.iter().any(|r| r.path.starts_with(t.p("home/.local")) && r.reason == "outside_tool_roots"), "{:?}", tools.refused);
}

#[test]
fn an_npm_whose_node_is_not_in_its_prefix_is_refused() {
    let t = Tree::new("npm-split");
    let cli = t.file("brew/lib/node_modules/npm/bin/npm-cli.js");
    t.link("brew/bin/npm", &cli);
    let evil = t.file("home/bin/node");
    t.link("brew/bin/node", &evil);
    let tools = resolve(&t.roots(), &[t.p("brew/bin")]);
    assert!(tools.npm.is_none());
    assert!(tools.refused.iter().any(|r| r.reason == "node_outside_prefix"), "{:?}", tools.refused);
}

#[test]
fn an_npm_link_that_is_not_npm_cli_js_is_refused() {
    let t = Tree::new("npm-shape");
    t.file("brew/bin/node");
    t.file("brew/bin/npm");
    let tools = resolve(&t.roots(), &[t.p("brew/bin")]);
    assert!(tools.npm.is_none());
    assert!(tools.refused.iter().any(|r| r.reason == "not_npm_cli"), "{:?}", tools.refused);
}

#[test]
fn an_nvm_node_counts_only_for_the_version_dir_the_login_path_names_and_is_labelled() {
    let t = Tree::new("nvm");
    t.node("home/.nvm/versions/node/v22.1.0");
    t.node("home/.nvm/versions/node/v20.0.0");
    let named = resolve(&t.roots(), &[t.p("home/.nvm/versions/node/v22.1.0/bin")]);
    let n = named.npm.expect("nvm npm on the PATH");
    assert_eq!(n.prefix, t.p("home/.nvm/versions/node/v22.1.0"));
    assert_eq!((n.node.label, n.cli.label), (Some("nvm"), Some("nvm")));
    assert!(resolve(&t.roots(), &[]).npm.is_none(), "an nvm dir the PATH does not name is never used");
}

#[test]
fn a_volta_node_is_install_it_yourself() {
    let t = Tree::new("volta");
    t.node("home/.volta/tools/image/node/22.0.0");
    t.link("home/.volta/bin/npm", &t.p("home/.volta/tools/image/node/22.0.0/lib/node_modules/npm/bin/npm-cli.js"));
    let tools = resolve(&t.roots(), &[t.p("home/.volta/bin")]);
    assert!(tools.npm.is_none());
    assert!(tools.refused.iter().any(|r| r.reason == "volta"), "{:?}", tools.refused);
}

#[test]
fn a_system_root_tool_is_used_and_not_labelled() {
    let t = Tree::new("sys");
    t.node("sys");
    let n = resolve(&t.roots(), &[t.p("sys/bin")]).npm.expect("system npm");
    assert_eq!(n.prefix, t.p("sys"));
    assert_eq!(n.node.label, None);
}
