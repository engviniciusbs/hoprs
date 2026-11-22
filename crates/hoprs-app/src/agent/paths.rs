//! Local paths an agent names, and the protected ones.
//!
//! An agent's `download` and `upload` name a path on this computer. Everything is open
//! except the [`Protected`] roots: hoprs's own directories ([`fixed_roots`]), the user's
//! `~/.ssh`, the `known_hosts` in use and every key file a saved site names. Without
//! them a download could overwrite `known_hosts` or `sites.toml`, and an upload could
//! hand a private key to the agent.
//!
//! The check compares whole components of two forms of each path: [`resolved`] (links
//! followed, through the nearest existing ancestor for a path that does not exist yet)
//! and the plain absolute path with `.` and `..` folded. A difference in letter case
//! counts as the same name on Windows and macOS, whose default filesystems ignore it
//! ([`CASE_INSENSITIVE`]); on a case-sensitive macOS volume that refuses a few paths
//! that are in fact different, the safe side. Elsewhere names compare exactly.
//!
//! Nothing here prints a root: the roots include every saved site's key file, so
//! [`Protected`]'s `Debug` shows only how many there are, and a refusal names only the
//! path the agent gave.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use hoprs_core::RemotePath;

/// Whether names compare without letter case: the default filesystems of Windows (NTFS)
/// and macOS (APFS) ignore it. Exact elsewhere.
const CASE_INSENSITIVE: bool = cfg!(any(windows, target_os = "macos"));

/// The most symbolic links [`resolved`] follows by hand (dangling ones, which
/// `canonicalize` refuses), like the systems' own limit of 40.
const LINK_HOPS: usize = 40;

#[cfg(not(windows))]
const NOT_ABSOLUTE: &str = "The local path must be absolute (start with /).";
#[cfg(windows)]
const NOT_ABSOLUTE: &str = r"The local path must be absolute, with a drive letter (C:\...).";
const DOTS: &str = "The local path must not contain . or .. components.";
const NOT_UNICODE: &str = "The local path must be valid Unicode.";
#[cfg(windows)]
const OTHER_DRIVE: &str =
    "The local path is on another drive: hoprs reaches only the drive it runs from.";
#[cfg(windows)]
const NETWORK: &str =
    "Network (UNC) and device paths are not supported: use a path on the drive hoprs runs from.";
#[cfg(windows)]
const BAD_NAME: &str = "The local path has a name Windows does not allow (with : or /, or ending in a dot or a space).";

/// The local paths an agent's transfer may not touch.
///
/// `Debug` prints only the number of roots (they include key files' paths).
#[derive(Clone, Default)]
pub struct Protected {
    roots: Vec<PathBuf>,
}

impl fmt::Debug for Protected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Protected")
            .field("roots", &self.roots.len())
            .finish()
    }
}

impl Protected {
    /// Protects `roots` (files or directories; they need not exist). An empty path is
    /// ignored, a relative one is read from the current folder at each check.
    // TODO(Task 8): remove
    #[allow(dead_code)]
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Protected {
        Protected {
            roots: roots
                .into_iter()
                .filter(|root| !root.as_os_str().is_empty())
                .collect(),
        }
    }

    /// `Err(path)` (as given) when `path` is a protected root, is inside one, or, with
    /// `tree` (a folder transfer), contains one. The roots are resolved again at each
    /// call, so a root created or linked since counts. Touches the disk: run it on the
    /// blocking pool.
    // TODO(Task 8): remove
    #[allow(dead_code)]
    pub fn check(&self, path: &Path, tree: bool) -> Result<(), PathBuf> {
        let candidate = forms(path);
        // A path with no form at all (empty) is not one hoprs can tell apart: refused.
        let hit = candidate.is_empty()
            || self.roots.iter().any(|root| {
                forms(root).iter().any(|root| {
                    candidate
                        .iter()
                        .any(|form| form.starts_with(root) || (tree && root.starts_with(form)))
                })
            });
        if hit { Err(path.to_path_buf()) } else { Ok(()) }
    }
}

/// `path` with its symbolic links resolved: `canonicalize` of its nearest existing
/// ancestor, the missing rest appended with `.` and `..` folded. A dangling link on the
/// way is followed to where writing through it would land. Never fails: when nothing
/// resolves, the absolute path with `.` and `..` folded.
// TODO(Task 8): remove
#[allow(dead_code)]
pub fn resolved(path: &Path) -> PathBuf {
    resolve(path, 0)
}

fn resolve(path: &Path, hops: usize) -> PathBuf {
    let absolute = absolute(path);
    let comps: Vec<Component<'_>> = absolute.components().collect();
    for end in (1..=comps.len()).rev() {
        let head: PathBuf = comps[..end].iter().collect();
        let rest = &comps[end..];
        if let Ok(canonical) = fs::canonicalize(&head) {
            return folded_onto(canonical, rest);
        }
        // `canonicalize` refuses a dangling link, but a download through it would
        // create its target.
        if hops < LINK_HOPS
            && fs::symlink_metadata(&head).is_ok_and(|meta| meta.file_type().is_symlink())
            && let Ok(target) = fs::read_link(&head)
        {
            let mut next = match head.parent() {
                Some(parent) => parent.join(target),
                None => target,
            };
            for comp in rest {
                next.push(comp);
            }
            return resolve(&next, hops + 1);
        }
    }
    folded(&absolute)
}

/// `std::path::absolute`, or `path` itself when that fails (an empty path).
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// `path` with `.` dropped and each `..` taking off the component before it, without
/// touching the disk.
fn folded(path: &Path) -> PathBuf {
    let comps: Vec<Component<'_>> = path.components().collect();
    folded_onto(PathBuf::new(), &comps)
}

fn folded_onto(mut out: PathBuf, rest: &[Component<'_>]) -> PathBuf {
    for comp in rest {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The forms of `path` a check compares, each as its comparable components:
/// [`resolved`] and the folded absolute path (one when both agree, none when empty).
fn forms(path: &Path) -> Vec<Vec<OsString>> {
    let mut out: Vec<Vec<OsString>> = Vec::with_capacity(2);
    for form in [resolved(path), folded(&absolute(path))] {
        let key = key(&form);
        if !key.is_empty() && !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

/// `path`'s components as compared: the drive or share in one spelling, names in lower
/// case where the system ignores case.
fn key(path: &Path) -> Vec<OsString> {
    path.components()
        .filter_map(|comp| match comp {
            Component::Prefix(prefix) => Some(prefix_key(prefix)),
            Component::RootDir => Some(OsString::from("/")),
            Component::CurDir => None,
            Component::ParentDir => Some(OsString::from("..")),
            Component::Normal(name) => name_key(name),
        })
        .collect()
}

fn name_key(name: &OsStr) -> Option<OsString> {
    if !CASE_INSENSITIVE {
        return Some(name.to_os_string());
    }
    let text = name.to_string_lossy().to_lowercase();
    #[cfg(windows)]
    let text = windows_name(text);
    (!text.is_empty()).then(|| OsString::from(text))
}

/// The name Windows opens for `name`: an alternate data stream (`file:stream`,
/// `file::$DATA`) is the file, and trailing dots and spaces are dropped.
#[cfg(windows)]
fn windows_name(mut name: String) -> String {
    if let Some(colon) = name.find(':') {
        name.truncate(colon);
    }
    let kept = name.trim_end_matches(['.', ' ']).len();
    name.truncate(kept);
    name
}

/// `C:` and `\\?\C:` are one drive, `\\server\share` and `\\?\UNC\server\share` one
/// share.
#[cfg(windows)]
fn prefix_key(prefix: std::path::PrefixComponent<'_>) -> OsString {
    use std::path::Prefix;
    match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
            OsString::from(format!("{}:", char::from(drive.to_ascii_uppercase())))
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => OsString::from(format!(
            r"\\{}\{}",
            server.to_string_lossy().to_lowercase(),
            share.to_string_lossy().to_lowercase()
        )),
        Prefix::Verbatim(_) | Prefix::DeviceNS(_) => {
            OsString::from(prefix.as_os_str().to_string_lossy().to_lowercase())
        }
    }
}

#[cfg(not(windows))]
fn prefix_key(prefix: std::path::PrefixComponent<'_>) -> OsString {
    prefix.as_os_str().to_os_string()
}

/// The roots every check protects, before the saved sites' key files (ruling 16):
/// `<base>/hoprs` for each standard base this system has (configuration, data, cache,
/// runtime, temporary), `<home>/.ssh`, `known_hosts` and `own` (the backend's own
/// directories).
// TODO(Task 8): remove
#[allow(dead_code)]
pub fn fixed_roots(own: &[PathBuf], known_hosts: &Path) -> Vec<PathBuf> {
    let bases = [
        dirs::config_dir(),
        dirs::data_dir(),
        dirs::cache_dir(),
        dirs::runtime_dir(),
        Some(std::env::temp_dir()),
    ];
    let mut roots: Vec<PathBuf> = bases
        .into_iter()
        .flatten()
        .map(|base| base.join("hoprs"))
        .collect();
    roots.extend(dirs::home_dir().map(|home| home.join(".ssh")));
    roots.push(known_hosts.to_path_buf());
    roots.extend(own.iter().cloned());
    roots
}

/// An agent's local path as the transfer engine's local side names it (ruling 15): the
/// side is `LocalFs::new("/")`, so the path must be absolute, with no `.` or `..`
/// component and in Unicode; on Windows it must also be on the current drive (another
/// drive, a network share and a device path are out of reach) and every name one Windows
/// can hold. The `Err` is the English sentence the tool answers. [`local_os_path`] is
/// the inverse, for the checks.
// TODO(Task 11): remove
#[allow(dead_code)]
pub fn engine_path(path: &Path) -> Result<RemotePath, &'static str> {
    let mut comps = path.components();
    #[cfg(windows)]
    {
        use std::path::Prefix;
        let Some(Component::Prefix(prefix)) = comps.next() else {
            return Err(NOT_ABSOLUTE);
        };
        match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                if current_drive() != Some(drive.to_ascii_uppercase()) {
                    return Err(OTHER_DRIVE);
                }
            }
            Prefix::UNC(..)
            | Prefix::VerbatimUNC(..)
            | Prefix::Verbatim(_)
            | Prefix::DeviceNS(_) => {
                return Err(NETWORK);
            }
        }
    }
    if comps.next() != Some(Component::RootDir) {
        return Err(NOT_ABSOLUTE);
    }
    let mut out = String::from("/");
    for comp in comps {
        match comp {
            Component::Normal(name) => {
                let name = name.to_str().ok_or(NOT_UNICODE)?;
                #[cfg(windows)]
                if name.contains([':', '/']) || name.ends_with(['.', ' ']) {
                    return Err(BAD_NAME);
                }
                if !out.ends_with('/') {
                    out.push('/');
                }
                out.push_str(name);
            }
            Component::CurDir | Component::ParentDir => return Err(DOTS),
            Component::Prefix(_) | Component::RootDir => return Err(NOT_ABSOLUTE),
        }
    }
    RemotePath::new(&out).map_err(|_| NOT_ABSOLUTE)
}

/// The drive `LocalFs::new("/")` reaches: the current folder's, upper case. `None` when
/// the current folder is not on a drive (a network share): then no drive is reachable.
#[cfg(windows)]
fn current_drive() -> Option<u8> {
    use std::path::Prefix;
    match std::path::absolute("/").ok()?.components().next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => Some(drive.to_ascii_uppercase()),
            _ => None,
        },
        _ => None,
    }
}

/// The file the local side opens for `path`, as the system writes it: its components
/// pushed onto the root, as `LocalFs::new("/")` does, made absolute without touching the
/// disk (on Windows that adds the current drive: `/Users/me` is `C:\Users\me`).
pub fn local_os_path(path: &RemotePath) -> PathBuf {
    let mut out = PathBuf::from("/");
    for comp in path.as_str().split('/').filter(|s| !s.is_empty()) {
        out.push(comp);
    }
    std::path::absolute(&out).unwrap_or(out)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::MAIN_SEPARATOR;

    use super::*;

    /// A tempdir holding `conf/hoprs/sites.toml`, `home/.ssh/known_hosts`,
    /// `keys/deploy.pem`, `keys/other.pem` and `work/a/`, with `conf/hoprs`, `home/.ssh`
    /// and `keys/deploy.pem` protected.
    struct Tree {
        dir: tempfile::TempDir,
        protected: Protected,
    }

    impl Tree {
        fn new() -> Tree {
            let dir = tempfile::tempdir().unwrap();
            let base = dir.path();
            for folder in ["conf/hoprs", "home/.ssh", "keys", "work/a"] {
                fs::create_dir_all(base.join(folder)).unwrap();
            }
            for file in [
                "conf/hoprs/sites.toml",
                "home/.ssh/known_hosts",
                "keys/deploy.pem",
                "keys/other.pem",
            ] {
                fs::write(base.join(file), b"x").unwrap();
            }
            let protected = Protected::new([
                base.join("conf").join("hoprs"),
                base.join("home").join(".ssh"),
                base.join("keys").join("deploy.pem"),
            ]);
            Tree { dir, protected }
        }

        /// `rel` (written with `/`) under the tempdir, built with `Path::join`.
        fn at(&self, rel: &str) -> PathBuf {
            rel.split('/')
                .filter(|s| !s.is_empty())
                .fold(self.dir.path().to_path_buf(), |path, comp| path.join(comp))
        }

        fn refused(&self, path: &Path, tree: bool) -> bool {
            match self.protected.check(path, tree) {
                Ok(()) => false,
                Err(given) => {
                    assert_eq!(given, path, "the refusal names the path as given");
                    true
                }
            }
        }
    }

    #[test]
    fn each_root_is_refused_as_itself_inside_and_around() {
        let t = Tree::new();
        for (root, inside, parent) in [
            ("conf/hoprs", "conf/hoprs/sites.toml", "conf"),
            ("home/.ssh", "home/.ssh/known_hosts", "home"),
            ("keys/deploy.pem", "keys/deploy.pem/x", "keys"),
        ] {
            for tree in [false, true] {
                assert!(t.refused(&t.at(root), tree), "{root} itself, tree {tree}");
                assert!(t.refused(&t.at(inside), tree), "{inside}, tree {tree}");
            }
            assert!(t.refused(&t.at(parent), true), "{parent} holds {root}");
            assert!(!t.refused(&t.at(parent), false), "{parent} as a file");
        }
        // The tempdir itself holds all three.
        assert!(t.refused(t.dir.path(), true));
        assert!(!t.refused(t.dir.path(), false));
    }

    #[test]
    fn neighbours_stay_open() {
        let t = Tree::new();
        for rel in [
            "conf/hoprs-notes/x",
            "home/.sshx",
            "home/.ssh2/key",
            "keys/other.pem",
            "keys/deploy.pem.bak",
            "work/a/b",
            "work",
        ] {
            for tree in [false, true] {
                assert!(!t.refused(&t.at(rel), tree), "{rel}, tree {tree}");
            }
        }
    }

    #[test]
    fn a_new_file_resolves_through_its_nearest_parent() {
        let t = Tree::new();
        assert!(t.refused(&t.at("conf/hoprs/new/deep/file"), false));
        assert!(!t.refused(&t.at("work/new/deep/file"), false));
        assert!(!t.refused(&t.at("work/new/deep/file"), true));
        // `resolved` of a missing path is its nearest existing parent's canonical form
        // with the rest appended.
        let canonical_work = fs::canonicalize(t.at("work")).unwrap();
        assert_eq!(
            resolved(&t.at("work/new/deep/file")),
            canonical_work.join("new").join("deep").join("file")
        );
        assert_eq!(resolved(&t.at("work/a")), canonical_work.join("a"));
    }

    #[test]
    fn dots_and_separators_do_not_walk_around_a_root() {
        let t = Tree::new();
        let sep = MAIN_SEPARATOR;
        let base = t.dir.path().display().to_string();
        for text in [
            format!("{base}{sep}work{sep}..{sep}conf{sep}hoprs{sep}sites.toml"),
            format!("{base}{sep}work{sep}new{sep}..{sep}..{sep}home{sep}.ssh{sep}x"),
            format!("{base}{sep}work{sep}a{sep}..{sep}..{sep}keys{sep}deploy.pem"),
            format!("{base}{sep}.{sep}conf{sep}.{sep}hoprs"),
            format!("{base}{sep}conf{sep}hoprs{sep}"),
            format!("{base}{sep}{sep}home{sep}{sep}.ssh{sep}{sep}known_hosts"),
            format!("{base}{sep}keys{sep}deploy.pem{sep}"),
        ] {
            assert!(t.refused(Path::new(&text), false), "{text}");
        }
        // Around: `..` from inside a root out to a neighbour is a neighbour.
        let out = format!("{base}{sep}conf{sep}hoprs{sep}..{sep}hoprs-notes{sep}x");
        assert!(!t.refused(Path::new(&out), true), "{out}");
        // `tree` sees a root under a folder written with dots.
        let around = format!("{base}{sep}work{sep}..{sep}home");
        assert!(t.refused(Path::new(&around), true), "{around}");
        assert!(!t.refused(Path::new(&around), false), "{around}");
    }

    #[test]
    fn a_relative_path_is_read_from_the_current_folder() {
        let t = Tree::new();
        let cwd = std::env::current_dir().unwrap();
        let protected = Protected::new([cwd.join("protected-here")]);
        assert!(
            protected
                .check(Path::new("protected-here/x"), false)
                .is_err()
        );
        assert!(protected.check(Path::new("."), true).is_err());
        assert!(protected.check(Path::new("elsewhere"), true).is_ok());
        // And nothing protects everything: an empty root is ignored.
        let empty = Protected::new([PathBuf::new()]);
        assert!(empty.check(&t.at("work/a"), true).is_ok());
        assert!(
            Protected::default()
                .check(&t.at("conf/hoprs"), true)
                .is_ok()
        );
        // An empty candidate names nothing hoprs can tell apart: refused.
        assert!(Protected::default().check(Path::new(""), false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_resolved() {
        use std::os::unix::fs::symlink;

        let t = Tree::new();
        symlink(t.at("home/.ssh"), t.at("work/link")).unwrap();
        assert!(t.refused(&t.at("work/link"), false));
        assert!(t.refused(&t.at("work/link/known_hosts"), false));
        assert!(t.refused(&t.at("work/link/new/file"), false));
        // `tree` looks at the folder's own path, not at the links inside it: the engine
        // never follows a link it lists, and its guard checks every item it opens.
        assert!(!t.refused(&t.at("work"), true));

        symlink(t.at("work/a"), t.at("work/alink")).unwrap();
        assert!(!t.refused(&t.at("work/alink"), true));
        assert!(!t.refused(&t.at("work/alink/b"), false));

        // A root that is itself reached through a link.
        symlink(t.at("conf"), t.at("alias")).unwrap();
        assert!(t.refused(&t.at("alias/hoprs/sites.toml"), false));
        assert!(t.refused(&t.at("alias/hoprs"), false));
        assert!(t.refused(&t.at("alias"), true));
        assert!(!t.refused(&t.at("alias/hoprs-notes"), false));
        // And a root named through a link protects its real path.
        let through = Protected::new([t.at("alias/hoprs")]);
        assert!(
            through
                .check(&t.at("conf/hoprs/sites.toml"), false)
                .is_err()
        );
        assert!(through.check(&t.at("conf"), true).is_err());
        assert!(through.check(&t.at("conf/hoprs-notes"), true).is_ok());

        // A key file reached through a relative link.
        symlink(Path::new("..").join("keys"), t.at("work/keys")).unwrap();
        assert!(t.refused(&t.at("work/keys/deploy.pem"), false));
        assert!(!t.refused(&t.at("work/keys/other.pem"), false));
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_link_is_followed_to_where_it_would_write() {
        use std::os::unix::fs::symlink;

        let t = Tree::new();
        // `work/new` does not exist yet: writing to `work/dangling` would create
        // `home/.ssh/authorized_keys`.
        symlink(t.at("home/.ssh/authorized_keys"), t.at("work/dangling")).unwrap();
        assert!(t.refused(&t.at("work/dangling"), false));
        symlink(
            Path::new("..")
                .join("home")
                .join(".ssh")
                .join("new")
                .join("x"),
            t.at("work/dangling-rel"),
        )
        .unwrap();
        assert!(t.refused(&t.at("work/dangling-rel"), false));
        symlink(t.at("work/nowhere"), t.at("work/dangling-open")).unwrap();
        assert!(!t.refused(&t.at("work/dangling-open"), false));
        // A loop of links resolves to something and does not hang.
        symlink(t.at("work/loop2"), t.at("work/loop1")).unwrap();
        symlink(t.at("work/loop1"), t.at("work/loop2")).unwrap();
        assert!(!t.refused(&t.at("work/loop1/x"), false));
    }

    #[test]
    fn letter_case() {
        let t = Tree::new();
        let insensitive = cfg!(any(windows, target_os = "macos"));
        for rel in ["home/.SSH/Known_Hosts", "CONF/Hoprs", "keys/Deploy.PEM"] {
            assert_eq!(t.refused(&t.at(rel), false), insensitive, "{rel}");
        }
        assert_eq!(t.refused(&t.at("Home"), true), insensitive);
    }

    #[cfg(windows)]
    #[test]
    fn windows_spellings_of_one_path() {
        let t = Tree::new();
        let base = fs::canonicalize(t.dir.path()).unwrap();
        // `canonicalize` answers the verbatim form, `\\?\C:\…`.
        assert!(base.display().to_string().starts_with(r"\\?\"), "{base:?}");
        let verbatim = base.join("home").join(".ssh").join("known_hosts");
        assert!(t.refused(&verbatim, false));
        let plain = std::path::absolute(t.dir.path()).unwrap();
        let text = plain.display().to_string();
        let forward = format!("{}/conf/hoprs/sites.toml", text.replace('\\', "/"));
        assert!(t.refused(Path::new(&forward), false), "{forward}");
        // Lower-case drive letter.
        let lower = format!("{}{}", text[..1].to_lowercase(), &text[1..]);
        assert!(
            t.refused(&Path::new(&lower).join("keys").join("deploy.pem"), false),
            "{lower}"
        );
        // A trailing dot or space and an alternate data stream name the same file.
        for odd in [
            r"home\.ssh.\known_hosts",
            r"home\.ssh \known_hosts",
            r"home\.ssh\known_hosts::$DATA",
            r"keys\deploy.pem:stream",
        ] {
            let path = PathBuf::from(format!(r"{text}\{odd}"));
            assert!(t.refused(&path, false), "{path:?}");
        }
    }

    #[test]
    fn the_roots_never_show_in_debug() {
        let t = Tree::new();
        let text = format!("{:?}", t.protected);
        assert_eq!(text, "Protected { roots: 3 }");
        assert!(!text.contains("deploy"), "{text}");
    }

    #[test]
    fn fixed_roots_cover_every_hoprs_directory() {
        let own = vec![
            PathBuf::from("/own/profile"),
            PathBuf::from("/own/queue"),
            PathBuf::from("/own/edit"),
        ];
        let known_hosts = PathBuf::from("/elsewhere/known_hosts");
        let roots: BTreeSet<PathBuf> = fixed_roots(&own, &known_hosts).into_iter().collect();

        let mut expected: BTreeSet<PathBuf> = [
            dirs::config_dir(),
            dirs::data_dir(),
            dirs::cache_dir(),
            dirs::runtime_dir(),
            Some(std::env::temp_dir()),
        ]
        .into_iter()
        .flatten()
        .map(|base| base.join("hoprs"))
        .collect();
        expected.extend(dirs::home_dir().map(|home| home.join(".ssh")));
        expected.insert(known_hosts.clone());
        expected.extend(own.iter().cloned());
        assert_eq!(roots, expected);
        assert!(roots.contains(&std::env::temp_dir().join("hoprs")));
    }

    #[test]
    fn local_os_path_is_the_native_path() {
        let path = RemotePath::new("/home/me/a b.txt").unwrap();
        let os = local_os_path(&path);
        assert!(os.is_absolute(), "{os:?}");
        assert!(os.ends_with(Path::new("home").join("me").join("a b.txt")));
        #[cfg(unix)]
        assert_eq!(os, PathBuf::from("/home/me/a b.txt"));
        // On Windows: a drive, backslashes (`C:\home\me\a b.txt`).
        #[cfg(windows)]
        {
            let text = os.display().to_string();
            assert!(text.ends_with(r"\home\me\a b.txt"), "{text}");
            assert_eq!(text.chars().nth(1), Some(':'), "{text}");
        }
        assert_eq!(
            local_os_path(&RemotePath::root()),
            std::path::absolute("/").unwrap()
        );
    }

    #[test]
    fn engine_path_rules() {
        assert!(engine_path(Path::new("relative/x")).is_err());
        assert!(engine_path(Path::new("")).is_err());
        assert!(engine_path(Path::new("x")).is_err());
        // Whatever the system, the inverse gives back the path the engine opens. Under
        // the current folder, which is on the current drive (on Windows runners the
        // temporary folder may be on another one).
        let abs = std::env::current_dir()
            .unwrap()
            .join("work")
            .join("b c.txt");
        let remote = engine_path(&abs).unwrap();
        assert_eq!(local_os_path(&remote), abs);
        let dotted = abs.parent().unwrap().join("..").join("x");
        assert!(engine_path(&dotted).is_err(), "{dotted:?}");
    }

    #[cfg(unix)]
    #[test]
    fn engine_path_rules_unix() {
        assert_eq!(
            engine_path(Path::new("/home/me/a b.txt")).unwrap(),
            RemotePath::new("/home/me/a b.txt").unwrap()
        );
        assert_eq!(
            engine_path(Path::new("//home//me/./x/")).unwrap(),
            RemotePath::new("/home/me/x").unwrap()
        );
        assert_eq!(engine_path(Path::new("/")).unwrap(), RemotePath::root());
        for bad in ["/home/me/../x", "/..", "../x", "./x"] {
            assert!(engine_path(Path::new(bad)).is_err(), "{bad}");
        }
        let sentence = engine_path(Path::new("relative/x")).unwrap_err();
        assert!(sentence.ends_with('.'), "{sentence}");
        assert!(engine_path(Path::new("/a/..")).unwrap_err().contains(".."));
        use std::os::unix::ffi::OsStrExt;
        let not_utf8 = Path::new(std::ffi::OsStr::from_bytes(b"/home/\xff"));
        assert!(engine_path(not_utf8).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn engine_path_rules_windows() {
        let current = std::path::absolute("/").unwrap().display().to_string();
        let drive = current.chars().next().unwrap();
        let other = if drive.eq_ignore_ascii_case(&'Q') {
            'R'
        } else {
            'Q'
        };
        let expected = RemotePath::new("/Users/me/a.txt").unwrap();
        for good in [
            format!(r"{drive}:\Users\me\a.txt"),
            format!("{drive}:/Users/me/a.txt"),
            format!(r"{}:\Users\me\a.txt", drive.to_ascii_lowercase()),
            format!(r"\\?\{drive}:\Users\me\a.txt"),
        ] {
            assert_eq!(
                engine_path(Path::new(&good)).as_ref(),
                Ok(&expected),
                "{good}"
            );
        }
        for bad in [
            format!(r"{other}:\Users\me\a.txt"),
            format!(r"\\?\{other}:\Users\me\a.txt"),
            r"\\server\share\x".to_owned(),
            r"\\?\UNC\server\share\x".to_owned(),
            r"\\.\pipe\x".to_owned(),
            r"\\?\GLOBALROOT\Device\HarddiskVolume1\x".to_owned(),
            r"Users\me".to_owned(),
            r"\Users\me".to_owned(),
            format!(r"{drive}:Users\me"),
            format!(r"{drive}:\Users\me\..\x"),
            format!(r"{drive}:\Users\me\a.txt:stream"),
            format!(r"\\?\{drive}:\Users\me/x"),
            format!(r"{drive}:\Users\me.\x"),
            format!(r"{drive}:\Users\me \x"),
        ] {
            assert!(engine_path(Path::new(&bad)).is_err(), "{bad}");
        }
    }
}
