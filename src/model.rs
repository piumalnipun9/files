//! File entries, classification, sorting and display formatting.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub hidden: bool,
    pub size: u64,
    pub mtime: i64,
    pub ext: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Cat {
    Folder,
    Image,
    Video,
    Audio,
    Pdf,
    Doc,
    Sheet,
    Slides,
    Archive,
    Code,
    Text,
    Exec,
    Font,
    Disk,
    Generic,
}

impl Entry {
    pub fn from_path(path: &Path) -> Option<Entry> {
        let name = path.file_name()?.to_string_lossy().into_owned();
        let lmeta = std::fs::symlink_metadata(path).ok()?;
        let is_link = lmeta.file_type().is_symlink();
        let meta = if is_link { std::fs::metadata(path).unwrap_or(lmeta) } else { lmeta };
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        Some(Entry {
            hidden: name.starts_with('.'),
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            mtime: meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0),
            name,
            path: path.to_path_buf(),
            ext,
        })
    }

    pub fn cat(&self) -> Cat {
        if self.is_dir {
            return Cat::Folder;
        }
        classify_ext(&self.ext, &self.name)
    }

    pub fn kind(&self) -> String {
        if self.is_dir {
            return "Folder".into();
        }
        let e = self.ext.to_uppercase();
        match self.cat() {
            Cat::Image => format!("{} image", if e.is_empty() { "Image".into() } else { e }),
            Cat::Video => format!("{} video", e),
            Cat::Audio => format!("{} audio", e),
            Cat::Pdf => "PDF document".into(),
            Cat::Doc => "Document".into(),
            Cat::Sheet => "Spreadsheet".into(),
            Cat::Slides => "Presentation".into(),
            Cat::Archive => format!("{} archive", e),
            Cat::Code => format!("{} source", if e.is_empty() { "Source".into() } else { e }),
            Cat::Text => "Plain text".into(),
            Cat::Exec => "Executable".into(),
            Cat::Font => "Font".into(),
            Cat::Disk => "Disk image".into(),
            _ => if e.is_empty() { "Document".into() } else { format!("{e} file") },
        }
    }
}

pub fn classify_ext(ext: &str, name: &str) -> Cat {
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "svg" | "avif" | "heic" | "ico" | "jxl" => Cat::Image,
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "flv" | "wmv" => Cat::Video,
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" | "aac" | "wma" | "oga" => Cat::Audio,
        "pdf" => Cat::Pdf,
        "doc" | "docx" | "odt" | "rtf" | "pages" => Cat::Doc,
        "xls" | "xlsx" | "ods" | "csv" | "tsv" | "numbers" => Cat::Sheet,
        "ppt" | "pptx" | "odp" | "key" => Cat::Slides,
        "zip" | "tar" | "gz" | "xz" | "bz2" | "zst" | "7z" | "rar" | "tgz" | "lz4" | "pkg" => Cat::Archive,
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "c" | "h" | "cpp" | "hpp" | "cc" | "go" | "java" | "kt" | "rb" | "php" | "lua"
        | "sh" | "zsh" | "bash" | "fish" | "html" | "css" | "scss" | "json" | "toml" | "yaml" | "yml" | "xml" | "qml" | "nix"
        | "sql" | "swift" | "zig" | "vue" | "svelte" | "conf" | "ini" | "cfg" => Cat::Code,
        "txt" | "md" | "log" | "markdown" | "rst" | "org" | "tex" => Cat::Text,
        "appimage" | "bin" | "run" | "exe" | "desktop" | "service" => Cat::Exec,
        "ttf" | "otf" | "woff" | "woff2" => Cat::Font,
        "iso" | "img" | "qcow2" | "vdi" => Cat::Disk,
        _ => {
            let n = name.to_lowercase();
            if matches!(n.as_str(), "makefile" | "dockerfile" | "cargo.lock" | "license" | "readme" | "pkgbuild") {
                Cat::Text
            } else {
                Cat::Generic
            }
        }
    }
}

/// Read a directory. Blocking — call from a worker thread.
pub fn read_dir(path: &Path) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(path).map_err(|e| e.to_string())?;
    let mut v = Vec::new();
    for ent in rd.flatten() {
        if let Some(e) = Entry::from_path(&ent.path()) {
            v.push(e);
        }
    }
    Ok(v)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Modified,
    Size,
    Kind,
}

pub fn sort_entries(v: &mut [Entry], key: SortKey, asc: bool) {
    v.sort_by(|a, b| {
        // Folders always first, like Finder / Files.
        match (a.is_dir, b.is_dir) {
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            _ => {}
        }
        let o = match key {
            SortKey::Name => natural_cmp(&a.name, &b.name),
            SortKey::Modified => a.mtime.cmp(&b.mtime),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Kind => a.ext.cmp(&b.ext).then_with(|| natural_cmp(&a.name, &b.name)),
        };
        if asc { o } else { o.reverse() }
    });
}

/// Case-insensitive comparison that orders digit runs numerically.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut nx = String::new();
                let mut ny = String::new();
                while let Some(c) = ai.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nx.push(c);
                    ai.next();
                }
                while let Some(c) = bi.peek().copied().filter(|c| c.is_ascii_digit()) {
                    ny.push(c);
                    bi.next();
                }
                let (tx, ty) = (nx.trim_start_matches('0'), ny.trim_start_matches('0'));
                let o = tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

pub fn fmt_size(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if b < 1000 {
        return format!("{b} B");
    }
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < 4 {
        v /= 1000.0;
        i += 1;
    }
    if v >= 100.0 { format!("{:.0} {}", v, U[i]) } else { format!("{:.1} {}", v, U[i]) }
}

/// "Today, 14:05" / "Yesterday, 09:12" / "Mar 3, 2026".
pub fn fmt_time(ts: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(t) = Local.timestamp_opt(ts, 0).single() else { return String::new() };
    let now = Local::now();
    let delta = (now.date_naive() - t.date_naive()).num_days();
    let clock = t.format("%H:%M");
    match delta {
        0 => format!("Today, {clock}"),
        1 => format!("Yesterday, {clock}"),
        _ if t.year() == now.year() => t.format("%b %-d, %H:%M").to_string(),
        _ => t.format("%b %-d, %Y").to_string(),
    }
}

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

pub fn trash_dir() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"));
    data.join("Trash")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["img10.png", "img2.png", "Img1.png", "a.txt"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a.txt", "Img1.png", "img2.png", "img10.png"]);
    }

    #[test]
    fn sizes_and_classes() {
        assert_eq!(fmt_size(999), "999 B");
        assert_eq!(fmt_size(1_500_000), "1.5 MB");
        assert_eq!(classify_ext("rs", "main.rs"), Cat::Code);
        assert_eq!(classify_ext("mkv", "a.mkv"), Cat::Video);
        assert_eq!(classify_ext("", "Makefile"), Cat::Text);
    }
}
