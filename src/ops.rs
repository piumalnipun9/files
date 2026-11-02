//! File operations. Everything here is blocking: run it in a worker thread.

use crate::model::trash_dir;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// `dir/name`, or `dir/name copy`, `dir/name copy 2`, … if taken.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let p = Path::new(name);
    let (stem, ext) = match (p.file_stem(), p.extension()) {
        (Some(s), Some(e)) if !name.starts_with('.') || name[1..].contains('.') => {
            (s.to_string_lossy().into_owned(), format!(".{}", e.to_string_lossy()))
        }
        _ => (name.to_string(), String::new()),
    };
    for n in 1.. {
        let cand = if n == 1 { format!("{stem} copy{ext}") } else { format!("{stem} copy {n}{ext}") };
        let c = dir.join(cand);
        if fs::symlink_metadata(&c).is_err() {
            return c;
        }
    }
    unreachable!()
}

pub fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(src)?, dst)
    } else if meta.is_dir() {
        fs::create_dir(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_recursive(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, dst).map(|_| ())
    }
}

pub fn remove_recursive(p: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(p)?;
    if meta.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }
}

/// Copy `src` into `dir`; returns the new path.
pub fn copy_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = src.file_name().ok_or_else(|| io::Error::other("no file name"))?.to_string_lossy().into_owned();
    if src.is_dir() && dir.starts_with(src) {
        return Err(io::Error::other("can't copy a folder into itself"));
    }
    let dst = unique_path(dir, &name);
    copy_recursive(src, &dst)?;
    Ok(dst)
}

/// Move `src` into `dir`; returns the new path. Falls back to copy+delete across filesystems.
pub fn move_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = src.file_name().ok_or_else(|| io::Error::other("no file name"))?.to_string_lossy().into_owned();
    if src.parent() == Some(dir) {
        return Ok(src.to_path_buf());
    }
    if src.is_dir() && dir.starts_with(src) {
        return Err(io::Error::other("can't move a folder into itself"));
    }
    let dst = unique_path(dir, &name);
    match fs::rename(src, &dst) {
        Ok(()) => Ok(dst),
        Err(_) => {
            copy_recursive(src, &dst)?;
            remove_recursive(src)?;
            Ok(dst)
        }
    }
}

/// Move to the freedesktop.org trash (same filesystem as $HOME assumed; falls back to copy+delete).
pub fn trash(p: &Path) -> Result<PathBuf, String> {
    let p = std::path::absolute(p).map_err(|e| e.to_string())?;
    let name = p.file_name().ok_or("no file name")?.to_string_lossy().into_owned();
    let (files, info) = (trash_dir().join("files"), trash_dir().join("info"));
    fs::create_dir_all(&files).map_err(|e| e.to_string())?;
    fs::create_dir_all(&info).map_err(|e| e.to_string())?;
    let dst = unique_path(&files, &name);
    let dname = dst.file_name().unwrap().to_string_lossy().into_owned();
    let date = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
    let body = format!("[Trash Info]\nPath={}\nDeletionDate={date}\n", percent_encode(&p.to_string_lossy()));
    fs::write(info.join(format!("{dname}.trashinfo")), body).map_err(|e| e.to_string())?;
    if fs::rename(&p, &dst).is_err() {
        copy_recursive(&p, &dst).map_err(|e| e.to_string())?;
        remove_recursive(&p).map_err(|e| e.to_string())?;
    }
    Ok(dst)
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Original location recorded in the .trashinfo of a trashed item.
pub fn trash_origin(p: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(info_path(p)?).ok()?;
    text.lines().find_map(|l| l.strip_prefix("Path=")).map(|o| PathBuf::from(percent_decode(o)))
}

fn info_path(p: &Path) -> Option<PathBuf> {
    let name = p.file_name()?.to_string_lossy().into_owned();
    Some(trash_dir().join("info").join(format!("{name}.trashinfo")))
}

/// Delete a trashed item for good (and its .trashinfo).
pub fn purge(p: &Path) -> io::Result<()> {
    remove_recursive(p)?;
    if let Some(i) = info_path(p) {
        let _ = fs::remove_file(i);
    }
    Ok(())
}

pub fn empty_trash() -> io::Result<usize> {
    let mut n = 0;
    let files = trash_dir().join("files");
    if let Ok(rd) = fs::read_dir(&files) {
        for e in rd.flatten() {
            purge(&e.path())?;
            n += 1;
        }
    }
    Ok(n)
}

/// Put a trashed item back where it came from.
pub fn restore(p: &Path) -> Result<PathBuf, String> {
    let info = info_path(p).ok_or("bad path")?;
    let text = fs::read_to_string(&info).map_err(|_| "no restore information for this item".to_string())?;
    let orig = text
        .lines()
        .find_map(|l| l.strip_prefix("Path="))
        .ok_or("no original location recorded")?;
    let orig = PathBuf::from(percent_decode(orig));
    let dir = orig.parent().ok_or("bad original location")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = orig.file_name().ok_or("bad original name")?.to_string_lossy().into_owned();
    let dst = unique_path(dir, &name);
    fs::rename(p, &dst).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(info);
    Ok(dst)
}

pub fn count_children(p: &Path) -> usize {
    fs::read_dir(p).map(|r| r.count()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("files-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn percent_round_trip() {
        let p = "/home/a b/ünï%.txt";
        assert_eq!(percent_decode(&percent_encode(p)), p);
    }

    #[test]
    fn unique_names() {
        let d = scratch("unique");
        fs::write(d.join("a.txt"), "x").unwrap();
        assert_eq!(unique_path(&d, "a.txt"), d.join("a copy.txt"));
        fs::write(d.join("a copy.txt"), "x").unwrap();
        assert_eq!(unique_path(&d, "a.txt"), d.join("a copy 2.txt"));
        assert_eq!(unique_path(&d, "new.txt"), d.join("new.txt"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_and_move_trees() {
        let d = scratch("tree");
        fs::create_dir_all(d.join("src/sub")).unwrap();
        fs::write(d.join("src/sub/f.txt"), "hello").unwrap();
        fs::create_dir(d.join("dst")).unwrap();
        let c = copy_into(&d.join("src"), &d.join("dst")).unwrap();
        assert_eq!(fs::read_to_string(c.join("sub/f.txt")).unwrap(), "hello");
        let m = move_into(&d.join("src"), &d.join("dst")).unwrap();
        assert_eq!(m, d.join("dst/src copy"));
        assert!(!d.join("src").exists());
        assert!(copy_into(&d.join("dst"), &d.join("dst/src copy")).is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
