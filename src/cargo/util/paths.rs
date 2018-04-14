use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::io::prelude::*;
use std::path::{Component, Path, PathBuf};
use std::iter;

use filetime::FileTime;

use util::{internal, CargoResult};
use util::errors::{CargoError, CargoResultExt, Internal};

pub fn join_paths<T: AsRef<OsStr>>(paths: &[T], env: &str) -> CargoResult<OsString> {
    let err = match env::join_paths(paths.iter()) {
        Ok(paths) => return Ok(paths),
        Err(e) => e,
    };
    let paths = paths.iter().map(Path::new).collect::<Vec<_>>();
    let err = CargoError::from(err);
    let explain = Internal::new(format_err!("failed to join path array: {:?}", paths));
    let err = CargoError::from(err.context(explain));
    let more_explain = format!(
        "failed to join search paths together\n\
         Does ${} have an unterminated quote character?",
        env
    );
    Err(err.context(more_explain).into())
}

pub fn dylib_path_envvar() -> &'static str {
    if cfg!(windows) {
        "PATH"
    } else if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    }
}

pub fn dylib_path() -> Vec<PathBuf> {
    match env::var_os(dylib_path_envvar()) {
        Some(var) => env::split_paths(&var).collect(),
        None => Vec::new(),
    }
}

pub fn normalize_path(path: &Path) -> PathBuf {
    let mut components = path.components().peekable();
    let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek().cloned() {
        components.next();
        PathBuf::from(c.as_os_str())
    } else {
        PathBuf::new()
    };

    for component in components {
        match component {
            Component::Prefix(..) => unreachable!(),
            Component::RootDir => {
                ret.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                ret.pop();
            }
            Component::Normal(c) => {
                ret.push(c);
            }
        }
    }
    ret
}

pub fn without_prefix<'a>(long_path: &'a Path, prefix: &'a Path) -> Option<&'a Path> {
    let mut a = long_path.components();
    let mut b = prefix.components();
    loop {
        match b.next() {
            Some(y) => match a.next() {
                Some(x) if x == y => continue,
                _ => return None,
            },
            None => return Some(a.as_path()),
        }
    }
}

pub fn resolve_executable(exec: &Path) -> CargoResult<PathBuf> {
    if exec.components().count() == 1 {
        let paths = env::var_os("PATH").ok_or(format_err!("no PATH"))?;
        let candidates = env::split_paths(&paths).flat_map(|path| {
            let candidate = PathBuf::from(path).join(&exec);
            let with_exe = if env::consts::EXE_EXTENSION == "" {
                None
            } else {
                Some(candidate.with_extension(env::consts::EXE_EXTENSION))
            };
            iter::once(candidate).chain(with_exe)
        });
        for candidate in candidates {
            if candidate.is_file() {
                // PATH may have a component like "." in it, so we still need to
                // canonicalize.
                return Ok(candidate.canonicalize()?);
            }
        }

        bail!("no executable for `{}` found in PATH", exec.display())
    } else {
        Ok(exec.canonicalize()?)
    }
}

pub fn read(path: &Path) -> CargoResult<String> {
    match String::from_utf8(read_bytes(path)?) {
        Ok(s) => Ok(s),
        Err(_) => bail!("path at `{}` was not valid utf-8", path.display()),
    }
}

pub fn read_bytes(path: &Path) -> CargoResult<Vec<u8>> {
    let res = (|| -> CargoResult<_> {
        let mut ret = Vec::new();
        let mut f = File::open(path)?;
        if let Ok(m) = f.metadata() {
            ret.reserve(m.len() as usize + 1);
        }
        f.read_to_end(&mut ret)?;
        Ok(ret)
    })()
        .chain_err(|| format!("failed to read `{}`", path.display()))?;
    Ok(res)
}

pub fn write(path: &Path, contents: &[u8]) -> CargoResult<()> {
    (|| -> CargoResult<()> {
        let mut f = File::create(path)?;
        f.write_all(contents)?;
        Ok(())
    })()
        .chain_err(|| format!("failed to write `{}`", path.display()))?;
    Ok(())
}

pub fn append(path: &Path, contents: &[u8]) -> CargoResult<()> {
    (|| -> CargoResult<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .append(true)
            .create(true)
            .open(path)?;

        f.write_all(contents)?;
        Ok(())
    })()
        .chain_err(|| internal(format!("failed to write `{}`", path.display())))?;
    Ok(())
}

pub fn mtime(path: &Path) -> CargoResult<FileTime> {
    let meta =
        fs::metadata(path).chain_err(|| internal(format!("failed to stat `{}`", path.display())))?;
    Ok(FileTime::from_last_modification_time(&meta))
}

#[cfg(unix)]
pub fn path2bytes(path: &Path) -> CargoResult<&[u8]> {
    use std::os::unix::prelude::*;
    Ok(path.as_os_str().as_bytes())
}
#[cfg(windows)]
pub fn path2bytes(path: &Path) -> CargoResult<&[u8]> {
    match path.as_os_str().to_str() {
        Some(s) => Ok(s.as_bytes()),
        None => Err(format_err!("invalid non-unicode path: {}", path.display())),
    }
}

#[cfg(unix)]
pub fn bytes2path(bytes: &[u8]) -> CargoResult<PathBuf> {
    use std::os::unix::prelude::*;
    use std::ffi::OsStr;
    Ok(PathBuf::from(OsStr::from_bytes(bytes)))
}
#[cfg(windows)]
pub fn bytes2path(bytes: &[u8]) -> CargoResult<PathBuf> {
    use std::str;
    match str::from_utf8(bytes) {
        Ok(s) => Ok(PathBuf::from(s)),
        Err(..) => Err(format_err!("invalid non-unicode path")),
    }
}

pub fn ancestors(path: &Path) -> PathAncestors {
    PathAncestors::new(path)
}

pub struct PathAncestors<'a> {
    current: Option<&'a Path>,
    stop_at: Option<PathBuf>,
}

impl<'a> PathAncestors<'a> {
    fn new(path: &Path) -> PathAncestors {
        PathAncestors {
            current: Some(path),
            //HACK: avoid reading `~/.cargo/config` when testing Cargo itself.
            stop_at: env::var("__CARGO_TEST_ROOT").ok().map(PathBuf::from),
        }
    }
}

impl<'a> Iterator for PathAncestors<'a> {
    type Item = &'a Path;

    fn next(&mut self) -> Option<&'a Path> {
        if let Some(path) = self.current {
            self.current = path.parent();

            if let Some(ref stop_at) = self.stop_at {
                if path == stop_at {
                    self.current = None;
                }
            }

            Some(path)
        } else {
            None
        }
    }
}

pub fn remove_dir_all<P: AsRef<Path>>(p: P) -> CargoResult<()> {
    _remove_dir_all(p.as_ref())
}

fn _remove_dir_all(p: &Path) -> CargoResult<()> {
    if p.symlink_metadata()?.file_type().is_symlink() {
        return remove_file(p);
    }
    let entries = p.read_dir()
        .chain_err(|| format!("failed to read directory `{}`", p.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            remove_dir_all(&path)?;
        } else {
            remove_file(&path)?;
        }
    }
    remove_dir(&p)
}

pub fn remove_dir<P: AsRef<Path>>(p: P) -> CargoResult<()> {
    _remove_dir(p.as_ref())
}

fn _remove_dir(p: &Path) -> CargoResult<()> {
    fs::remove_dir(p).chain_err(|| format!("failed to remove directory `{}`", p.display()))?;
    Ok(())
}

pub fn remove_file<P: AsRef<Path>>(p: P) -> CargoResult<()> {
    _remove_file(p.as_ref())
}

fn _remove_file(p: &Path) -> CargoResult<()> {
    let mut err = match fs::remove_file(p) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };

    if err.kind() == io::ErrorKind::PermissionDenied && set_not_readonly(p).unwrap_or(false) {
        match fs::remove_file(p) {
            Ok(()) => return Ok(()),
            Err(e) => err = e,
        }
    }

    Err(err).chain_err(|| format!("failed to remove file `{}`", p.display()))?;
    Ok(())
}

fn set_not_readonly(p: &Path) -> io::Result<bool> {
    let mut perms = p.metadata()?.permissions();
    if !perms.readonly() {
        return Ok(false);
    }
    perms.set_readonly(false);
    fs::set_permissions(p, perms)?;
    Ok(true)
}
