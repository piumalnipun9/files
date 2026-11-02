//! The controller: owns navigation, the file list, selection, sidebar and menus,
//! and pushes everything into the Slint window.

use crate::model::{self, Cat, Entry, SortKey};
use crate::theme::{self, Tints};
use crate::thumbs::{self, Thumbs};
use crate::{AppWindow, Crumb, Details, FileItem, MenuEntry, SideRow};
use slint::{Image, Model, ModelRc, SharedString, Timer, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

/// Material Symbols codepoints used from Rust (the Slint side has its own `Sym` global).
pub mod sym {
    pub const HOME: &str = "\u{e88a}";
    pub const FOLDER: &str = "\u{e2c7}";
    pub const FOLDER_OPEN: &str = "\u{e2c8}";
    pub const DESKTOP: &str = "\u{e30c}";
    pub const DOWNLOAD: &str = "\u{e171}";
    pub const DESCRIPTION: &str = "\u{e873}";
    pub const IMAGE: &str = "\u{e251}";
    pub const MUSIC: &str = "\u{e3a1}";
    pub const MOVIE: &str = "\u{e02c}";
    pub const PUBLIC: &str = "\u{e80b}";
    pub const DRAFT: &str = "\u{e06f}";
    pub const CODE: &str = "\u{e86f}";
    pub const SCHEDULE: &str = "\u{e192}";
    pub const DELETE: &str = "\u{e872}";
    pub const DELETE_FOREVER: &str = "\u{e92b}";
    pub const RESTORE: &str = "\u{e938}";
    pub const HARD_DRIVE: &str = "\u{f80e}";
    pub const USB: &str = "\u{e1e0}";
    pub const PDF: &str = "\u{e415}";
    pub const VIDEO_FILE: &str = "\u{eb87}";
    pub const AUDIO_FILE: &str = "\u{eb82}";
    pub const ARTICLE: &str = "\u{ef42}";
    pub const TABLE: &str = "\u{e265}";
    pub const SLIDES: &str = "\u{e41b}";
    pub const ZIP: &str = "\u{eb2c}";
    pub const TEXT: &str = "\u{f1c6}";
    pub const TERMINAL: &str = "\u{eb8e}";
    pub const FONT: &str = "\u{e167}";
    pub const DISC: &str = "\u{e019}";
    pub const OPEN: &str = "\u{e895}";
    pub const CUT: &str = "\u{e14e}";
    pub const COPY: &str = "\u{e14d}";
    pub const PASTE: &str = "\u{e14f}";
    pub const RENAME: &str = "\u{e9a2}";
    pub const PIN: &str = "\u{e6aa}";
    pub const UNPIN: &str = "\u{e017}";
    pub const LIGHTBULB: &str = "\u{e0f0}";
    pub const NEW_FOLDER: &str = "\u{e2cc}";
    pub const NOTE_ADD: &str = "\u{e89c}";
    pub const SELECT_ALL: &str = "\u{e162}";
    pub const REFRESH: &str = "\u{e5d5}";
    pub const CHECK: &str = "\u{e5ca}";
    pub const UP: &str = "\u{e5d8}";
    pub const DOWN: &str = "\u{e5db}";
}

#[derive(Clone, PartialEq, Debug)]
pub enum Loc {
    Dir(PathBuf),
    Recent,
    Trash,
}

impl Loc {
    fn dir(&self) -> Option<PathBuf> {
        match self {
            Loc::Dir(p) => Some(p.clone()),
            Loc::Trash => Some(model::trash_dir().join("files")),
            Loc::Recent => None,
        }
    }
}

enum Dialog {
    Rename(PathBuf),
    NewFolder(PathBuf),
    NewFile(PathBuf),
}

struct SideMeta {
    key: String,
    path: Option<PathBuf>,
    y: f32,
}

pub struct State {
    loc: Loc,
    back: Vec<Loc>,
    forward: Vec<Loc>,
    all: Vec<Entry>,
    shown: Vec<Entry>,
    sel: Vec<bool>,
    anchor: Option<usize>,
    cursor: Option<usize>,
    search: String,
    // persisted
    view: i32,
    details: bool,
    sort: SortKey,
    asc: bool,
    hidden: bool,
    thumbnails: bool,
    ui_filled: bool,
    ui_glyphs: bool,
    ui_tiles: bool,
    home_open: bool,
    expanded: HashSet<PathBuf>,
    pins: Vec<PathBuf>,
    colors: HashMap<String, usize>,
    // sidebar
    side: Vec<SideMeta>,
    specials: Vec<(PathBuf, &'static str)>,
    // transient
    clipboard: Vec<PathBuf>,
    cut: bool,
    menu_target: Vec<PathBuf>,
    dialog: Option<Dialog>,
    undo: Vec<PathBuf>,
    snack_serial: u64,
    thumb_cache: HashMap<PathBuf, (i64, thumbs::Pixels)>,
    generation: u64,
    watch_stamp: Option<SystemTime>,
    theme_stamp: Option<SystemTime>,
    ticks: u64,
}

pub struct Ctl {
    pub app: AppWindow,
    st: RefCell<State>,
    items: Rc<VecModel<FileItem>>,
    thumbs: Thumbs,
}

thread_local! {
    static CTL: RefCell<Option<Rc<Ctl>>> = const { RefCell::new(None) };
}

/// Run `f` against the controller (UI thread only).
pub fn with(f: impl FnOnce(&Rc<Ctl>)) {
    let c = CTL.with(|c| c.borrow().clone());
    if let Some(c) = c {
        f(&c);
    }
}

fn settings_path() -> PathBuf {
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| model::home().join(".config"));
    cfg.join("files/state.json")
}

fn stamp(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn tilde(p: &Path) -> String {
    let home = model::home();
    match p.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

fn spawn_detached(mut cmd: std::process::Command) {
    use std::process::Stdio;
    if let Ok(mut child) = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// XDG user directories and the glyph drawn on their folder.
fn special_dirs() -> Vec<(PathBuf, &'static str)> {
    let home = model::home();
    let mut found: HashMap<&str, PathBuf> = HashMap::new();
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    if let Ok(text) = std::fs::read_to_string(cfg.join("user-dirs.dirs")) {
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
            let k = k.trim().trim_start_matches("XDG_").trim_end_matches("_DIR");
            for name in ["DESKTOP", "DOCUMENTS", "DOWNLOAD", "MUSIC", "PICTURES", "VIDEOS", "PUBLICSHARE", "TEMPLATES", "PROJECTS"] {
                if k == name && Path::new(&v) != home {
                    found.insert(name, PathBuf::from(&v));
                }
            }
        }
    }
    let order: [(&str, &str, &'static str); 9] = [
        ("DESKTOP", "Desktop", sym::DESKTOP),
        ("DOCUMENTS", "Documents", sym::DESCRIPTION),
        ("DOWNLOAD", "Downloads", sym::DOWNLOAD),
        ("PICTURES", "Pictures", sym::IMAGE),
        ("MUSIC", "Music", sym::MUSIC),
        ("VIDEOS", "Videos", sym::MOVIE),
        ("PROJECTS", "Projects", sym::CODE),
        ("PUBLICSHARE", "Public", sym::PUBLIC),
        ("TEMPLATES", "Templates", sym::DRAFT),
    ];
    order
        .iter()
        .map(|(k, fallback, glyph)| (found.get(k).cloned().unwrap_or_else(|| home.join(fallback)), *glyph))
        .collect()
}

fn cat_icon(cat: Cat) -> &'static str {
    match cat {
        Cat::Folder => sym::FOLDER,
        Cat::Image => sym::IMAGE,
        Cat::Video => sym::VIDEO_FILE,
        Cat::Audio => sym::AUDIO_FILE,
        Cat::Pdf => sym::PDF,
        Cat::Doc => sym::ARTICLE,
        Cat::Sheet => sym::TABLE,
        Cat::Slides => sym::SLIDES,
        Cat::Archive => sym::ZIP,
        Cat::Code => sym::CODE,
        Cat::Text => sym::TEXT,
        Cat::Exec => sym::TERMINAL,
        Cat::Font => sym::FONT,
        Cat::Disk => sym::DISC,
        Cat::Generic => sym::DRAFT,
    }
}

fn entry(id: &str, label: &str, icon: &str) -> MenuEntry {
    MenuEntry { id: id.into(), label: label.into(), icon: icon.into(), kind: 0, enabled: true, danger: false, hint: SharedString::new() }
}

trait MenuExt {
    fn hint(self, h: &str) -> Self;
    fn danger(self) -> Self;
    fn enabled(self, on: bool) -> Self;
}

impl MenuExt for MenuEntry {
    fn hint(mut self, h: &str) -> Self {
        self.hint = h.into();
        self
    }
    fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
}

fn divider() -> MenuEntry {
    MenuEntry { kind: 1, ..entry("", "", "") }
}

fn swatch_row() -> MenuEntry {
    MenuEntry { kind: 2, ..entry("", "", "") }
}

fn recent_entries() -> Vec<Entry> {
    let file = model::home().join(".local/share/recently-used.xbel");
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for chunk in text.split("<bookmark ").skip(1) {
        let attr = |name: &str| -> Option<String> {
            let start = chunk.find(&format!("{name}=\""))? + name.len() + 2;
            let end = chunk[start..].find('"')? + start;
            Some(chunk[start..end].to_string())
        };
        let Some(href) = attr("href") else { continue };
        let Some(path) = href.strip_prefix("file://") else { continue };
        let when = attr("visited").or_else(|| attr("modified")).unwrap_or_default();
        found.push((when, PathBuf::from(crate::ops::percent_decode(path))));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter(|(_, p)| seen.insert(p.clone()))
        .filter_map(|(_, p)| Entry::from_path(&p))
        .filter(|e| !e.is_dir)
        .take(200)
        .collect()
}

fn storage() -> (String, f32) {
    let Ok(c) = std::ffi::CString::new(model::home().to_string_lossy().as_bytes()) else { return (String::new(), 0.0) };
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return (String::new(), 0.0);
    }
    let total = s.f_blocks as u64 * s.f_frsize as u64;
    let free = s.f_bavail as u64 * s.f_frsize as u64;
    let used = total.saturating_sub(free);
    let frac = if total > 0 { used as f32 / total as f32 } else { 0.0 };
    (format!("{} of {} used", model::fmt_size(used), model::fmt_size(total)), frac)
}

fn list_subdirs(p: &Path, hidden: bool) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(p) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| hidden || !e.file_name().to_string_lossy().starts_with('.'))
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    v.sort_by(|a, b| model::natural_cmp(&a.file_name().unwrap_or_default().to_string_lossy(), &b.file_name().unwrap_or_default().to_string_lossy()));
    v
}

/// Whether `p` has at least one (visible) subfolder; stops at the first one.
fn has_subdirs(p: &Path, hidden: bool) -> bool {
    let Ok(rd) = std::fs::read_dir(p) else { return false };
    rd.flatten().any(|e| {
        (hidden || !e.file_name().to_string_lossy().starts_with('.'))
            && e.file_type().map(|t| t.is_dir() || (t.is_symlink() && e.path().is_dir())).unwrap_or(false)
    })
}

fn mounts() -> Vec<PathBuf> {
    let user = std::env::var("USER").unwrap_or_default();
    let mut v = Vec::new();
    for base in [PathBuf::from("/run/media").join(&user), PathBuf::from("/media").join(&user), PathBuf::from("/mnt")] {
        v.extend(list_subdirs(&base, false));
    }
    v
}

impl Ctl {
    pub fn install(app: AppWindow) -> Rc<Ctl> {
        let items = Rc::new(VecModel::<FileItem>::default());
        app.set_items(ModelRc::from(items.clone()));
        let thumbs = Thumbs::new(|generation, path, px| {
            let _ = slint::invoke_from_event_loop(move || with(|c| c.thumb_ready(generation, path, px)));
        });
        let mut st = State {
            loc: Loc::Dir(model::home()),
            back: Vec::new(),
            forward: Vec::new(),
            all: Vec::new(),
            shown: Vec::new(),
            sel: Vec::new(),
            anchor: None,
            cursor: None,
            search: String::new(),
            view: 0,
            details: false,
            sort: SortKey::Name,
            asc: true,
            hidden: false,
            thumbnails: true,
            ui_filled: false,
            ui_glyphs: false,
            ui_tiles: false,
            home_open: true,
            expanded: HashSet::new(),
            pins: Vec::new(),
            colors: HashMap::new(),
            side: Vec::new(),
            specials: special_dirs(),
            clipboard: Vec::new(),
            cut: false,
            menu_target: Vec::new(),
            dialog: None,
            undo: Vec::new(),
            snack_serial: 0,
            thumb_cache: HashMap::new(),
            generation: 0,
            watch_stamp: None,
            theme_stamp: stamp(&theme::path()),
            ticks: 0,
        };
        load_settings(&mut st);
        theme::apply(&app);
        let ctl = Rc::new(Ctl { app, st: RefCell::new(st), items, thumbs });
        CTL.with(|c| *c.borrow_mut() = Some(ctl.clone()));
        ctl.wire();
        ctl.push_settings();
        ctl.refresh_storage();
        ctl
    }

    fn wire(&self) {
        let a = &self.app;
        a.on_side_activate(|k| with(|c| c.side_activate(&k)));
        a.on_side_toggle(|k| with(|c| c.side_toggle(&k)));
        a.on_side_context(|k, x, y| with(|c| c.side_context(&k, x, y)));
        a.on_new_menu(|x, y| with(|c| c.new_menu(x, y)));
        a.on_back(|| with(|c| c.history(true)));
        a.on_forward(|| with(|c| c.history(false)));
        a.on_up(|| with(|c| c.up()));
        a.on_crumb(|i, x, y| with(|c| c.crumb(i, x, y)));
        a.on_search_edited(|t| with(|c| c.search(&t)));
        a.on_search_escape(|| with(|c| c.search_escape()));
        a.on_set_view(|v| with(|c| c.set_view(v)));
        a.on_toggle_details(|| with(|c| c.toggle_details()));
        a.on_sort_menu(|x, y| with(|c| c.sort_menu(x, y)));
        a.on_toggle_hidden(|| with(|c| c.toggle_hidden()));
        a.on_press(|i, ctrl, shift| with(|c| c.press(i, ctrl, shift)));
        a.on_open(|i| with(|c| c.open_index(i as usize)));
        a.on_context(|i, x, y| with(|c| c.context(i, x, y)));
        a.on_menu_action(|id| with(|c| c.action(&id)));
        a.on_snack_act(|| with(|c| c.undo()));
        a.on_snack_dismiss(|| with(|c| c.app.set_snack_shown(false)));
        a.on_dialog_accept(|t| with(|c| c.dialog_accept(&t)));
        a.on_dialog_cancel(|| with(|c| c.dialog_close()));
        a.on_details_color(|i| with(|c| c.details_color(i as usize)));
        a.on_empty_trash(|| with(|c| c.action("empty-trash")));
        a.on_settings_menu(|x, y| with(|c| c.settings_menu(x, y)));
        a.on_key(|k, ctrl, shift, alt| {
            let mut handled = false;
            with(|c| handled = c.key(&k, ctrl, shift, alt));
            handled
        });
        let t = Timer::default();
        t.start(slint::TimerMode::Repeated, Duration::from_millis(1000), || with(|c| c.tick()));
        std::mem::forget(t);
    }

    // ---------- settings ----------

    fn push_settings(&self) {
        let st = self.st.borrow();
        self.app.set_view_mode(st.view);
        self.app.set_details_open(st.details);
        self.app.set_show_hidden(st.hidden);
        self.app.set_ui_filled_icons(st.ui_filled);
        self.app.set_ui_folder_glyphs(st.ui_glyphs);
        self.app.set_ui_tiles(st.ui_tiles);
        self.app.set_sort_key(sort_id(st.sort).into());
        self.app.set_sort_asc(st.asc);
        self.app.set_sort_label(sort_label(st.sort).into());
        self.app.set_swatches(ModelRc::new(VecModel::from(Tints::from_app(&self.app).swatches())));
    }

    fn save(&self) {
        let st = self.st.borrow();
        let v = serde_json::json!({
            "view": st.view,
            "details": st.details,
            "sort": sort_id(st.sort),
            "asc": st.asc,
            "hidden": st.hidden,
            "thumbnails": st.thumbnails,
            "ui_filled_icons": st.ui_filled,
            "ui_folder_glyphs": st.ui_glyphs,
            "ui_tiles": st.ui_tiles,
            "home_open": st.home_open,
            "expanded": st.expanded.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            "pins": st.pins.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            "colors": st.colors,
        });
        let path = settings_path();
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(path, serde_json::to_string_pretty(&v).unwrap_or_default());
    }

    // ---------- navigation ----------

    pub fn go(&self, loc: Loc) {
        let loc = match loc {
            Loc::Dir(p) if p == model::trash_dir().join("files") => Loc::Trash,
            other => other,
        };
        {
            let mut st = self.st.borrow_mut();
            if st.loc != loc {
                let prev = std::mem::replace(&mut st.loc, loc);
                st.back.push(prev);
                st.forward.clear();
            }
        }
        self.enter();
    }

    fn history(&self, back: bool) {
        {
            let mut st = self.st.borrow_mut();
            let next = if back { st.back.pop() } else { st.forward.pop() };
            let Some(next) = next else { return };
            let prev = std::mem::replace(&mut st.loc, next);
            if back { st.forward.push(prev) } else { st.back.push(prev) }
        }
        self.enter();
    }

    fn up(&self) {
        let parent = match &self.st.borrow().loc {
            Loc::Dir(p) => p.parent().map(Path::to_path_buf),
            _ => None,
        };
        if let Some(p) = parent {
            self.go(Loc::Dir(p));
        }
    }

    /// Fresh look at the current location: reset search/selection, reload, animate in.
    fn enter(&self) {
        {
            let mut st = self.st.borrow_mut();
            st.search.clear();
            st.anchor = None;
            st.cursor = None;
            st.sel.clear();
        }
        self.app.set_search_text("".into());
        self.reload(false);
        self.app.invoke_scroll_top();
        self.push_location();
        self.build_side();
    }

    fn push_location(&self) {
        let st = self.st.borrow();
        let crumbs = crumbs(&st.loc);
        let n = crumbs.len();
        let start = n.saturating_sub(4);
        let mut shown: Vec<Crumb> = Vec::new();
        for (i, (label, _)) in crumbs.iter().enumerate().skip(start) {
            let label = if i == start && start > 0 { "…".to_string() } else { label.clone() };
            shown.push(Crumb { label: label.into(), index: i as i32 });
        }
        let title = crumbs.last().map(|c| c.0.clone()).unwrap_or_default();
        self.app.set_crumbs(ModelRc::new(VecModel::from(shown)));
        self.app.set_title_text(title.clone().into());
        self.app.set_can_back(!st.back.is_empty());
        self.app.set_can_forward(!st.forward.is_empty());
        self.app.set_can_up(matches!(&st.loc, Loc::Dir(p) if p.parent().is_some()));
        self.app.set_in_trash(st.loc == Loc::Trash);
        self.app.set_search_placeholder(format!("Search in {title}").into());
    }

    fn crumb(&self, i: i32, x: f32, y: f32) {
        let (target, last) = {
            let st = self.st.borrow();
            let c = crumbs(&st.loc);
            (c.get(i as usize).and_then(|c| c.1.clone()), i as usize + 1 == c.len())
        };
        if last {
            self.context(-1, x, y);
        } else if let Some(p) = target {
            self.go(Loc::Dir(p));
        }
    }

    // ---------- listing ----------

    /// Re-read the location. `keep` preserves selection (by path) across the reload.
    fn reload(&self, keep: bool) {
        let loc = self.st.borrow().loc.clone();
        let (all, watch) = match &loc {
            Loc::Recent => (recent_entries(), stamp(&model::home().join(".local/share/recently-used.xbel"))),
            other => {
                let dir = other.dir().unwrap();
                let mut v = model::read_dir(&dir).unwrap_or_default();
                if *other == Loc::Trash {
                    // Show where trashed things came from as their "kind".
                    for e in &mut v {
                        e.hidden = false;
                    }
                }
                (v, stamp(&dir))
            }
        };
        let kept: HashSet<PathBuf> = if keep {
            let st = self.st.borrow();
            st.shown.iter().zip(&st.sel).filter(|(_, s)| **s).map(|(e, _)| e.path.clone()).collect()
        } else {
            HashSet::new()
        };
        {
            let mut st = self.st.borrow_mut();
            st.all = all;
            st.watch_stamp = watch;
        }
        self.refilter(&kept);
    }

    fn refilter(&self, keep: &HashSet<PathBuf>) {
        let generation = self.thumbs.bump();
        let tints = Tints::from_app(&self.app);
        let mut st = self.st.borrow_mut();
        st.generation = generation;
        let needle = st.search.to_lowercase();
        let mut shown: Vec<Entry> = st
            .all
            .iter()
            .filter(|e| st.hidden || !e.hidden)
            .filter(|e| needle.is_empty() || e.name.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        if st.loc == Loc::Recent {
            // Already in most-recent-first order.
        } else {
            model::sort_entries(&mut shown, st.sort, st.asc);
        }
        st.sel = shown.iter().map(|e| keep.contains(&e.path)).collect();
        st.shown = shown;
        let folders = st.shown.iter().filter(|e| e.is_dir).count();
        let rows: Vec<FileItem> = st.shown.iter().zip(&st.sel).map(|(e, s)| self.item(&st, &tints, e, *s)).collect();
        for e in &st.shown {
            if st.thumbnails && !e.is_dir && thumbs::supported(&e.ext) && !st.thumb_cache.get(&e.path).is_some_and(|(m, _)| *m == e.mtime) {
                self.thumbs.request(generation, e.path.clone());
            }
        }
        let (empty_title, empty_body) = match (&st.loc, st.search.is_empty()) {
            (_, false) => ("No results".to_string(), format!("Nothing here matches “{}”", st.search)),
            (Loc::Trash, _) => ("Trash is empty".into(), "Items moved to the trash appear here".into()),
            (Loc::Recent, _) => ("No recent files".into(), "Files you open will show up here".into()),
            _ => ("This folder is empty".into(), "Use New to create a folder or file".into()),
        };
        drop(st);
        self.items.set_vec(rows);
        self.app.set_folder_count(folders as i32);
        self.app.set_empty_title(empty_title.into());
        self.app.set_empty_body(empty_body.into());
        self.selection_changed();
    }

    fn item(&self, st: &State, tints: &Tints, e: &Entry, selected: bool) -> FileItem {
        let cat = e.cat();
        let key = e.path.to_string_lossy();
        let (tint, emblem) = if e.is_dir {
            let idx = st.colors.get(key.as_ref()).copied().unwrap_or(0);
            let emblem = st.specials.iter().find(|(p, _)| *p == e.path).map(|(_, g)| *g).unwrap_or("");
            (tints.swatch(idx), emblem)
        } else {
            (tints.for_cat(cat), "")
        };
        let cached = st.thumb_cache.get(&e.path).filter(|(m, _)| st.thumbnails && *m == e.mtime);
        let size = if e.is_dir {
            if st.shown.len() < 1500 {
                let n = crate::ops::count_children(&e.path);
                if n == 1 { "1 item".to_string() } else { format!("{n} items") }
            } else {
                String::new()
            }
        } else {
            model::fmt_size(e.size)
        };
        let kind = if st.loc == Loc::Trash {
            crate::ops::trash_origin(&e.path).and_then(|p| p.parent().map(tilde)).map(|p| format!("From {p}")).unwrap_or_else(|| e.kind())
        } else {
            e.kind()
        };
        FileItem {
            name: e.name.as_str().into(),
            kind: kind.into(),
            size: size.into(),
            modified: model::fmt_time(e.mtime).into(),
            icon: cat_icon(cat).into(),
            emblem: emblem.into(),
            is_dir: e.is_dir,
            tint,
            thumb: cached.map(|(_, px)| Image::from_rgba8(px.clone())).unwrap_or_default(),
            has_thumb: cached.is_some(),
            selected,
            cut: st.cut && st.clipboard.contains(&e.path),
        }
    }

    fn thumb_ready(&self, generation: u64, path: PathBuf, px: thumbs::Pixels) {
        let mut st = self.st.borrow_mut();
        let Some(idx) = st.shown.iter().position(|e| e.path == path) else { return };
        let mtime = st.shown[idx].mtime;
        if st.thumb_cache.len() > 400 {
            st.thumb_cache.clear();
        }
        st.thumb_cache.insert(path, (mtime, px.clone()));
        if generation != st.generation {
            return;
        }
        drop(st);
        if let Some(mut row) = self.items.row_data(idx) {
            row.thumb = Image::from_rgba8(px);
            row.has_thumb = true;
            self.items.set_row_data(idx, row);
        }
        if self.app.get_details_open() {
            self.update_details();
        }
    }

    /// Re-render rows in place (colours, cut state) without touching the list.
    fn restyle(&self) {
        let tints = Tints::from_app(&self.app);
        let st = self.st.borrow();
        let rows: Vec<FileItem> = st.shown.iter().zip(&st.sel).map(|(e, s)| self.item(&st, &tints, e, *s)).collect();
        drop(st);
        for (i, r) in rows.into_iter().enumerate() {
            self.items.set_row_data(i, r);
        }
        self.update_details();
    }

    fn search(&self, text: &str) {
        self.st.borrow_mut().search = text.to_string();
        self.refilter(&HashSet::new());
        self.app.invoke_scroll_top();
    }

    fn search_escape(&self) {
        self.app.set_search_text("".into());
        self.search("");
        self.app.invoke_focus_main();
    }

    fn set_view(&self, v: i32) {
        self.st.borrow_mut().view = v;
        self.app.set_view_mode(v);
        self.save();
    }

    fn toggle_details(&self) {
        let on = {
            let mut st = self.st.borrow_mut();
            st.details = !st.details;
            st.details
        };
        self.app.set_details_open(on);
        self.update_details();
        self.save();
    }

    fn resort(&self) {
        let keep: HashSet<PathBuf> = {
            let st = self.st.borrow();
            st.shown.iter().zip(&st.sel).filter(|(_, s)| **s).map(|(e, _)| e.path.clone()).collect()
        };
        self.refilter(&keep);
    }

    fn toggle_hidden(&self) {
        {
            let mut st = self.st.borrow_mut();
            st.hidden = !st.hidden;
        }
        self.push_settings();
        self.resort();
        self.build_side();
        self.save();
    }

    // ---------- selection ----------

    fn press(&self, i: i32, ctrl: bool, shift: bool) {
        {
            let mut st = self.st.borrow_mut();
            let n = st.sel.len();
            if i < 0 {
                if !ctrl {
                    st.sel.iter_mut().for_each(|s| *s = false);
                    st.anchor = None;
                }
            } else {
                let i = i as usize;
                if i >= n {
                    return;
                }
                if shift {
                    let a = st.anchor.unwrap_or(i);
                    let (lo, hi) = (a.min(i), a.max(i));
                    if !ctrl {
                        st.sel.iter_mut().for_each(|s| *s = false);
                    }
                    for j in lo..=hi {
                        st.sel[j] = true;
                    }
                } else if ctrl {
                    st.sel[i] = !st.sel[i];
                    st.anchor = Some(i);
                } else {
                    st.sel.iter_mut().for_each(|s| *s = false);
                    st.sel[i] = true;
                    st.anchor = Some(i);
                }
                st.cursor = Some(i);
            }
        }
        self.sync_selection();
    }

    /// Push `sel` into the rows that changed.
    fn sync_selection(&self) {
        let sel = self.st.borrow().sel.clone();
        for (i, s) in sel.iter().enumerate() {
            if let Some(mut row) = self.items.row_data(i) {
                if row.selected != *s {
                    row.selected = *s;
                    self.items.set_row_data(i, row);
                }
            }
        }
        self.selection_changed();
    }

    fn selection_changed(&self) {
        let n = self.st.borrow().sel.iter().filter(|s| **s).count();
        self.app.set_selection_count(n as i32);
        self.app.set_single_selection(n == 1);
        self.update_details();
    }

    fn selected(&self) -> Vec<PathBuf> {
        let st = self.st.borrow();
        st.shown.iter().zip(&st.sel).filter(|(_, s)| **s).map(|(e, _)| e.path.clone()).collect()
    }

    fn select_paths(&self, paths: &[PathBuf]) {
        let idx = {
            let mut st = self.st.borrow_mut();
            let sel: Vec<bool> = st.shown.iter().map(|e| paths.contains(&e.path)).collect();
            st.sel = sel;
            let first = st.sel.iter().position(|s| *s);
            st.anchor = first;
            st.cursor = first;
            first
        };
        self.sync_selection();
        if let Some(i) = idx {
            self.app.invoke_reveal(i as i32);
        }
    }

    fn update_details(&self) {
        if !self.app.get_details_open() {
            return;
        }
        let tints = Tints::from_app(&self.app);
        let st = self.st.borrow();
        let picked: Vec<usize> = st.sel.iter().enumerate().filter(|(_, s)| **s).map(|(i, _)| i).collect();
        let mut d = Details::default();
        match picked.as_slice() {
            [] => {
                // Describe the folder itself.
                let (name, icon, path) = match &st.loc {
                    Loc::Recent => ("Recent".to_string(), sym::SCHEDULE, None),
                    Loc::Trash => ("Trash".to_string(), sym::DELETE, None),
                    Loc::Dir(p) => (crumbs(&st.loc).last().map(|c| c.0.clone()).unwrap_or_default(), sym::FOLDER, Some(p.clone())),
                };
                let mut item = FileItem { name: name.into(), icon: icon.into(), tint: tints.primary, ..Default::default() };
                if let Some(p) = &path {
                    let idx = st.colors.get(p.to_string_lossy().as_ref()).copied().unwrap_or(0);
                    item.is_dir = true;
                    item.tint = tints.swatch(idx);
                    item.kind = "Folder".into();
                    item.emblem = st.specials.iter().find(|(s, _)| s == p).map(|(_, g)| *g).unwrap_or("").into();
                    if let Some(e) = Entry::from_path(p) {
                        item.modified = model::fmt_time(e.mtime).into();
                    }
                    d.location = p.parent().map(tilde).unwrap_or_default().into();
                    d.can_color = true;
                    d.color_index = idx as i32;
                } else {
                    item.kind = "Collection".into();
                }
                let n = st.shown.len();
                d.contents = format!("{n} item{}", if n == 1 { "" } else { "s" }).into();
                d.item = item;
            }
            [i] => {
                let e = &st.shown[*i];
                d.item = self.items.row_data(*i).unwrap_or_default();
                d.item.selected = false;
                d.contents = d.item.size.clone();
                d.location = e.path.parent().map(tilde).unwrap_or_default().into();
                if st.loc == Loc::Trash {
                    d.location = crate::ops::trash_origin(&e.path).and_then(|p| p.parent().map(tilde)).unwrap_or_default().into();
                    d.item.kind = e.kind().into();
                } else if e.is_dir {
                    d.can_color = true;
                    d.color_index = st.colors.get(e.path.to_string_lossy().as_ref()).copied().unwrap_or(0) as i32;
                }
            }
            many => {
                let bytes: u64 = many.iter().map(|i| st.shown[*i].size).sum();
                let all_dirs = many.iter().all(|i| st.shown[*i].is_dir);
                d.multiple = true;
                d.item = FileItem {
                    name: format!("{} items", many.len()).into(),
                    kind: if all_dirs { "Folders".into() } else { "Multiple types".into() },
                    size: model::fmt_size(bytes).into(),
                    icon: sym::COPY.into(),
                    tint: tints.primary,
                    ..Default::default()
                };
                d.location = st.loc.dir().map(|p| tilde(&p)).unwrap_or_default().into();
                d.can_color = all_dirs && st.loc != Loc::Trash;
            }
        }
        drop(st);
        self.app.set_details(d);
    }

    fn details_color(&self, idx: usize) {
        let targets = {
            let sel = self.selected();
            if sel.is_empty() { self.st.borrow().loc.dir().into_iter().collect() } else { sel }
        };
        self.set_colors(&targets, idx);
    }

    fn set_colors(&self, paths: &[PathBuf], idx: usize) {
        {
            let mut st = self.st.borrow_mut();
            for p in paths {
                let k = p.to_string_lossy().into_owned();
                if idx == 0 {
                    st.colors.remove(&k);
                } else {
                    st.colors.insert(k, idx);
                }
            }
        }
        self.restyle();
        self.build_side();
        self.save();
    }

    // ---------- opening ----------

    fn open_index(&self, i: usize) {
        let e = self.st.borrow().shown.get(i).cloned();
        if let Some(e) = e {
            self.open_entry(&e);
        }
    }

    fn open_entry(&self, e: &Entry) {
        if e.is_dir {
            self.go(Loc::Dir(e.path.clone()));
        } else {
            let mut cmd = std::process::Command::new("xdg-open");
            cmd.arg(&e.path);
            spawn_detached(cmd);
        }
    }

    fn open_selection(&self) {
        let picked: Vec<Entry> = {
            let st = self.st.borrow();
            st.shown.iter().zip(&st.sel).filter(|(_, s)| **s).map(|(e, _)| e.clone()).collect()
        };
        if let Some(d) = picked.iter().find(|e| e.is_dir) {
            if picked.len() == 1 {
                self.open_entry(d);
                return;
            }
        }
        for e in picked.iter().filter(|e| !e.is_dir) {
            self.open_entry(e);
        }
    }

    fn terminal(dir: &Path) {
        let term = std::env::var("TERMINAL").ok().or_else(|| {
            ["alacritty", "kitty", "foot", "ghostty", "wezterm", "konsole", "gnome-terminal", "xterm"]
                .into_iter()
                .find(|t| std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).any(|d| d.join(t).exists()))
                .map(String::from)
        });
        if let Some(t) = term {
            let mut cmd = std::process::Command::new(t);
            cmd.current_dir(dir);
            spawn_detached(cmd);
        }
    }

    // ---------- sidebar ----------

    fn build_side(&self) {
        let tints = Tints::from_app(&self.app);
        let mut st = self.st.borrow_mut();
        let home = model::home();
        let mut rows: Vec<SideRow> = Vec::new();
        let mut meta: Vec<SideMeta> = Vec::new();
        let mut y = 0.0f32;
        let mut push = |rows: &mut Vec<SideRow>, meta: &mut Vec<SideMeta>, mut r: SideRow, path: Option<PathBuf>| {
            let h = match r.kind {
                2 => 16.0,
                _ => 32.0,
            };
            r.y = y;
            r.height = h;
            meta.push(SideMeta { key: r.key.to_string(), path, y });
            rows.push(r);
            y += h;
        };
        let folder_color = |st: &State, p: &Path| st.colors.get(p.to_string_lossy().as_ref()).copied().unwrap_or(0);

        push(
            &mut rows,
            &mut meta,
            {
                let c = folder_color(&st, &home);
                SideRow {
                    kind: 0,
                    key: "home".into(),
                    label: "Home".into(),
                    icon: sym::HOME.into(),
                    expandable: true,
                    expanded: st.home_open,
                    tint: tints.swatch(c),
                    tinted: c > 0,
                    ..Default::default()
                }
            },
            Some(home.clone()),
        );
        if st.home_open {
            let mut dirs = list_subdirs(&home, false);
            let rank = |p: &PathBuf| st.specials.iter().position(|(s, _)| s == p).unwrap_or(usize::MAX);
            dirs.sort_by_key(rank);
            for d in dirs {
                let special = st.specials.iter().find(|(s, _)| *s == d).map(|(_, g)| *g);
                let c = folder_color(&st, &d);
                let r = SideRow {
                    kind: 0,
                    key: format!("dir:{}", d.display()).into(),
                    label: d.file_name().unwrap_or_default().to_string_lossy().as_ref().into(),
                    icon: special.unwrap_or(sym::FOLDER).into(),
                    open_icon: if special.is_some() { "" } else { sym::FOLDER_OPEN }.into(),
                    folder: true,
                    emblem: special.unwrap_or("").into(),
                    depth: 1,
                    tint: tints.swatch(c),
                    tinted: c > 0,
                    ..Default::default()
                };
                push(&mut rows, &mut meta, r, Some(d));
            }
            for p in st.pins.clone() {
                if !p.is_dir() {
                    continue;
                }
                let label = if p == Path::new("/") { "File system".to_string() } else { p.file_name().unwrap_or_default().to_string_lossy().into_owned() };
                let c = folder_color(&st, &p);
                let r = SideRow {
                    kind: 0,
                    key: format!("pin:{}", p.display()).into(),
                    label: label.into(),
                    icon: sym::FOLDER.into(),
                    open_icon: sym::FOLDER_OPEN.into(),
                    folder: true,
                    depth: 1,
                    tint: tints.swatch(c),
                    tinted: c > 0,
                    pinned: true,
                    ..Default::default()
                };
                push(&mut rows, &mut meta, r, Some(p));
            }
        }
        push(&mut rows, &mut meta, SideRow { kind: 2, ..Default::default() }, None);
        push(&mut rows, &mut meta, SideRow { kind: 0, key: "recent".into(), label: "Recent".into(), icon: sym::SCHEDULE.into(), ..Default::default() }, None);
        push(&mut rows, &mut meta, SideRow { kind: 0, key: "trash".into(), label: "Trash".into(), icon: sym::DELETE.into(), ..Default::default() }, None);
        push(&mut rows, &mut meta, SideRow { kind: 2, ..Default::default() }, None);
        push(&mut rows, &mut meta, SideRow { kind: 1, label: "System".into(), ..Default::default() }, None);

        // Expandable tree of the whole filesystem plus mounted drives.
        let mut roots: Vec<(PathBuf, String, &'static str)> = vec![(PathBuf::from("/"), "File system".into(), sym::HARD_DRIVE)];
        for m in mounts() {
            let name = m.file_name().unwrap_or_default().to_string_lossy().into_owned();
            roots.push((m, name, sym::USB));
        }
        let mut stack: Vec<(PathBuf, String, &'static str, i32)> = roots.into_iter().rev().map(|(p, n, g)| (p, n, g, 0)).collect();
        while let Some((p, label, glyph, depth)) = stack.pop() {
            let open = st.expanded.contains(&p);
            let c = folder_color(&st, &p);
            let r = SideRow {
                kind: 0,
                key: format!("tree:{}", p.display()).into(),
                label: label.into(),
                icon: glyph.into(),
                open_icon: if glyph == sym::FOLDER { sym::FOLDER_OPEN } else { "" }.into(),
                folder: glyph == sym::FOLDER,
                depth,
                expandable: has_subdirs(&p, st.hidden),
                expanded: open,
                tint: tints.swatch(c),
                tinted: c > 0,
                ..Default::default()
            };
            push(&mut rows, &mut meta, r, Some(p.clone()));
            if open {
                let mut kids = list_subdirs(&p, st.hidden);
                // The way home comes first (e.g. /home under /), then alphabetical.
                kids.sort_by_key(|k| !home.starts_with(k));
                for k in kids.into_iter().rev() {
                    let name = k.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    stack.push((k, name, sym::FOLDER, depth + 1));
                }
            }
        }

        st.side = meta;
        let selected = side_match(&st);
        drop(st);
        self.app.set_side_rows(ModelRc::new(VecModel::from(rows)));
        self.sync_side_selection(selected);
    }

    fn sync_side_selection(&self, found: Option<(String, f32)>) {
        match found {
            Some((key, _)) => {
                self.app.set_side_key(key.into());
                self.app.set_side_visible(true);
            }
            None => self.app.set_side_visible(false),
        }
    }

    fn side_path(&self, key: &str) -> Option<PathBuf> {
        self.st.borrow().side.iter().find(|m| m.key == key).and_then(|m| m.path.clone())
    }

    fn side_activate(&self, key: &str) {
        match key {
            "recent" => self.go(Loc::Recent),
            "trash" => self.go(Loc::Trash),
            _ => {
                if let Some(p) = self.side_path(key) {
                    // Pills slide to the row that was clicked, even when another row shares the path.
                    let y = self.st.borrow().side.iter().find(|m| m.key == key).map(|m| m.y);
                    self.go(Loc::Dir(p));
                    if let Some(y) = y {
                        self.sync_side_selection(Some((key.to_string(), y)));
                    }
                }
            }
        }
    }

    fn side_toggle(&self, key: &str) {
        {
            let path = self.side_path(key);
            let mut st = self.st.borrow_mut();
            if key == "home" {
                st.home_open = !st.home_open;
            } else if let Some(p) = path {
                if !st.expanded.remove(&p) {
                    st.expanded.insert(p);
                }
            }
        }
        self.build_side();
        self.save();
    }

    fn side_context(&self, key: &str, x: f32, y: f32) {
        let mut entries = Vec::new();
        if key == "trash" {
            entries.push(entry("empty-trash", "Empty trash", sym::DELETE_FOREVER).danger());
            self.st.borrow_mut().menu_target = Vec::new();
        } else if let Some(p) = self.side_path(key) {
            let pinned = self.st.borrow().pins.contains(&p);
            entries.push(entry("open-target", "Open", sym::FOLDER_OPEN));
            entries.push(entry("terminal-target", "Open terminal here", sym::TERMINAL));
            entries.push(divider());
            if key != "home" {
                entries.push(if pinned { entry("unpin", "Unpin from sidebar", sym::UNPIN) } else { entry("pin", "Pin to sidebar", sym::PIN) });
            }
            entries.push(swatch_row());
            let idx = self.st.borrow().colors.get(p.to_string_lossy().as_ref()).copied().unwrap_or(0);
            self.app.set_menu_swatch(idx as i32);
            self.st.borrow_mut().menu_target = vec![p];
        } else {
            return;
        }
        self.show_menu(entries, x, y);
    }

    // ---------- menus ----------

    fn show_menu(&self, entries: Vec<MenuEntry>, x: f32, y: f32) {
        let h: f32 = 16.0 + entries.iter().map(|e| match e.kind { 1 => 17.0, 2 => 46.0, 3 => 32.0, _ => 36.0 }).sum::<f32>();
        self.app.set_menu_height(h);
        self.app.set_menu_entries(ModelRc::new(VecModel::from(entries)));
        self.app.invoke_show_menu(x, y);
    }

    fn new_menu(&self, x: f32, y: f32) {
        let can = self.st.borrow().loc.dir().is_some() && self.st.borrow().loc != Loc::Trash;
        let entries = vec![
            entry("new-folder", "New folder", sym::NEW_FOLDER).hint("Ctrl+Shift+N").enabled(can),
            entry("new-file", "New text file", sym::NOTE_ADD).enabled(can),
            divider(),
            entry("paste", "Paste", sym::PASTE).hint("Ctrl+V").enabled(can && !self.st.borrow().clipboard.is_empty()),
        ];
        self.show_menu(entries, x, y);
    }

    fn settings_menu(&self, x: f32, y: f32) {
        let (hidden, thumbs, home_open, filled, glyphs, tiles) = {
            let st = self.st.borrow();
            (st.hidden, st.thumbnails, st.home_open, st.ui_filled, st.ui_glyphs, st.ui_tiles)
        };
        let check = |on: bool| if on { sym::CHECK } else { "" };
        let entries = vec![
            entry("hidden", "Show hidden files", check(hidden)).hint("Ctrl+H"),
            entry("thumbnails", "Show image previews", check(thumbs)),
            entry("home-open", "Expand Home in sidebar", check(home_open)),
            divider(),
            entry("reset-colors", "Reset folder colours", sym::RESTORE),
            entry("clear-pins", "Remove all pinned folders", sym::UNPIN),
            divider(),
            MenuEntry { kind: 3, ..entry("", "UI experiments", "") },
            entry("ui:filled", "Filled sidebar icons", check(filled)),
            entry("ui:glyphs", "Sidebar folders match main view", check(glyphs)),
            entry("ui:tiles", "Same-size grid tiles", check(tiles)),
        ];
        self.show_menu(entries, x, y);
    }

    fn sort_menu(&self, x: f32, y: f32) {
        let (sort, asc) = {
            let st = self.st.borrow();
            (st.sort, st.asc)
        };
        let mark = |k: SortKey, label: &str, id: &str| {
            let e = entry(id, label, if sort == k { sym::CHECK } else { "" });
            e
        };
        let entries = vec![
            mark(SortKey::Name, "Name", "sort:name"),
            mark(SortKey::Modified, "Date modified", "sort:modified"),
            mark(SortKey::Size, "Size", "sort:size"),
            mark(SortKey::Kind, "Kind", "sort:kind"),
            divider(),
            entry("asc", "Ascending", if asc { sym::UP } else { "" }),
            entry("desc", "Descending", if asc { "" } else { sym::DOWN }),
        ];
        self.show_menu(entries, x, y);
    }

    /// -1: folder background, -2: the selection bar's overflow, otherwise an item.
    fn context(&self, i: i32, x: f32, y: f32) {
        if i >= 0 {
            let needs = !self.st.borrow().sel.get(i as usize).copied().unwrap_or(true);
            if needs {
                self.press(i, false, false);
            }
        } else if i == -1 {
            self.press(-1, false, false);
        }
        let st = self.st.borrow();
        let picked: Vec<&Entry> = st.shown.iter().zip(&st.sel).filter(|(_, s)| **s).map(|(e, _)| e).collect();
        let has_clip = !st.clipboard.is_empty();
        let mut entries = Vec::new();
        if picked.is_empty() {
            match &st.loc {
                Loc::Trash => entries.push(entry("empty-trash", "Empty trash", sym::DELETE_FOREVER).danger().enabled(!st.shown.is_empty())),
                Loc::Recent => entries.push(entry("refresh", "Refresh", sym::REFRESH).hint("F5")),
                Loc::Dir(_) => {
                    entries.push(entry("new-folder", "New folder", sym::NEW_FOLDER).hint("Ctrl+Shift+N"));
                    entries.push(entry("new-file", "New text file", sym::NOTE_ADD));
                    entries.push(divider());
                    entries.push(entry("paste", "Paste", sym::PASTE).hint("Ctrl+V").enabled(has_clip));
                    entries.push(entry("select-all", "Select all", sym::SELECT_ALL).hint("Ctrl+A"));
                    entries.push(divider());
                    entries.push(entry("terminal", "Open terminal here", sym::TERMINAL));
                    entries.push(entry("refresh", "Refresh", sym::REFRESH).hint("F5"));
                }
            }
        } else if st.loc == Loc::Trash {
            entries.push(entry("restore", "Restore", sym::RESTORE));
            entries.push(entry("details", "Details", sym::LIGHTBULB));
            entries.push(divider());
            entries.push(entry("purge", "Delete forever", sym::DELETE_FOREVER).danger());
        } else {
            let single_dir = picked.len() == 1 && picked[0].is_dir;
            let all_dirs = picked.iter().all(|e| e.is_dir);
            entries.push(entry("open", "Open", sym::OPEN).hint("Enter"));
            if st.loc == Loc::Recent {
                entries.push(entry("reveal", "Show in folder", sym::FOLDER_OPEN));
            }
            if single_dir {
                let pinned = st.pins.contains(&picked[0].path);
                entries.push(if pinned { entry("unpin", "Unpin from sidebar", sym::UNPIN) } else { entry("pin", "Pin to sidebar", sym::PIN) });
            }
            if all_dirs {
                let idx = st.colors.get(picked[0].path.to_string_lossy().as_ref()).copied().unwrap_or(0);
                self.app.set_menu_swatch(idx as i32);
                entries.push(swatch_row());
            }
            entries.push(divider());
            entries.push(entry("cut", "Cut", sym::CUT).hint("Ctrl+X"));
            entries.push(entry("copy", "Copy", sym::COPY).hint("Ctrl+C"));
            if single_dir && has_clip {
                entries.push(entry("paste-into", "Paste into folder", sym::PASTE));
            }
            if picked.len() == 1 {
                entries.push(entry("rename", "Rename", sym::RENAME).hint("F2"));
            }
            entries.push(entry("details", "Details", sym::LIGHTBULB));
            entries.push(divider());
            entries.push(entry("trash", "Move to trash", sym::DELETE).hint("Del").danger());
        }
        let targets: Vec<PathBuf> = picked.iter().map(|e| e.path.clone()).collect();
        drop(st);
        self.st.borrow_mut().menu_target = targets;
        self.show_menu(entries, x, y);
    }

    pub fn action(&self, id: &str) {
        let here = self.st.borrow().loc.dir();
        let targets = self.st.borrow().menu_target.clone();
        let picked = {
            let s = self.selected();
            if s.is_empty() { targets.clone() } else { s }
        };
        if let Some(idx) = id.strip_prefix("color:") {
            let paths = if targets.is_empty() { picked } else { targets };
            self.set_colors(&paths, idx.parse().unwrap_or(0));
            return;
        }
        if let Some(k) = id.strip_prefix("sort:") {
            {
                let mut st = self.st.borrow_mut();
                st.sort = sort_from(k);
            }
            self.push_settings();
            self.resort();
            self.save();
            return;
        }
        match id {
            "asc" | "desc" => {
                self.st.borrow_mut().asc = id == "asc";
                self.push_settings();
                self.resort();
                self.save();
            }
            "deselect" => self.press(-1, false, false),
            "open" => self.open_selection(),
            "open-target" => {
                if let Some(p) = targets.first() {
                    self.go(Loc::Dir(p.clone()));
                }
            }
            "terminal-target" => {
                if let Some(p) = targets.first() {
                    Self::terminal(p);
                }
            }
            "terminal" => {
                if let Some(d) = here {
                    Self::terminal(&d);
                }
            }
            "reveal" => {
                if let Some(p) = picked.first() {
                    if let Some(parent) = p.parent() {
                        self.go(Loc::Dir(parent.to_path_buf()));
                        self.select_paths(&[p.clone()]);
                    }
                }
            }
            "pin" | "unpin" => {
                {
                    let mut st = self.st.borrow_mut();
                    for p in &targets {
                        st.pins.retain(|x| x != p);
                        if id == "pin" {
                            st.pins.push(p.clone());
                        }
                    }
                }
                self.build_side();
                self.save();
                self.snack(if id == "pin" { "Pinned to sidebar" } else { "Removed from sidebar" }, false);
            }
            "cut" | "copy" => {
                let n = picked.len();
                {
                    let mut st = self.st.borrow_mut();
                    st.clipboard = picked;
                    st.cut = id == "cut";
                }
                self.restyle();
                let verb = if id == "cut" { "cut" } else { "copied" };
                self.snack(&format!("{n} item{} {verb}", if n == 1 { "" } else { "s" }), false);
            }
            "paste" => {
                if let Some(d) = here {
                    self.paste(d);
                }
            }
            "paste-into" => {
                if let Some(d) = targets.first() {
                    self.paste(d.clone());
                }
            }
            "select-all" => {
                {
                    let mut st = self.st.borrow_mut();
                    st.sel.iter_mut().for_each(|s| *s = true);
                }
                self.sync_selection();
            }
            "hidden" => self.toggle_hidden(),
            "thumbnails" => {
                {
                    let mut st = self.st.borrow_mut();
                    st.thumbnails = !st.thumbnails;
                }
                self.resort();
                self.save();
            }
            "home-open" => self.side_toggle("home"),
            "ui:filled" | "ui:glyphs" | "ui:tiles" => {
                {
                    let mut st = self.st.borrow_mut();
                    let flag = match id {
                        "ui:filled" => &mut st.ui_filled,
                        "ui:glyphs" => &mut st.ui_glyphs,
                        _ => &mut st.ui_tiles,
                    };
                    *flag = !*flag;
                }
                self.push_settings();
                self.save();
            }
            "reset-colors" => {
                self.st.borrow_mut().colors.clear();
                self.restyle();
                self.build_side();
                self.save();
                self.snack("Folder colours reset", false);
            }
            "clear-pins" => {
                self.st.borrow_mut().pins.clear();
                self.build_side();
                self.save();
            }
            "refresh" => self.reload(true),
            "details" => {
                if !self.st.borrow().details {
                    self.toggle_details();
                }
            }
            "rename" => {
                if let Some(p) = picked.first() {
                    let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    let stem = match name.rfind('.') {
                        Some(i) if i > 0 && !p.is_dir() => i,
                        _ => name.chars().count(),
                    };
                    let stem = name[..stem].chars().count();
                    self.ask(Dialog::Rename(p.clone()), "Rename", &name, "OK", stem);
                }
            }
            "new-folder" => {
                if let Some(d) = here.filter(|_| self.st.borrow().loc != Loc::Trash) {
                    self.ask(Dialog::NewFolder(d), "New folder", "Untitled folder", "Create", usize::MAX);
                }
            }
            "new-file" => {
                if let Some(d) = here.filter(|_| self.st.borrow().loc != Loc::Trash) {
                    self.ask(Dialog::NewFile(d), "New text file", "Untitled.txt", "Create", 8);
                }
            }
            "trash" => self.trash(picked),
            "restore" => {
                let mut errs = 0;
                for p in &picked {
                    if crate::ops::restore(p).is_err() {
                        errs += 1;
                    }
                }
                self.reload(false);
                let n = picked.len() - errs;
                self.snack(&format!("{n} item{} restored", if n == 1 { "" } else { "s" }), false);
            }
            "purge" => {
                let n = picked.iter().filter(|p| crate::ops::purge(p).is_ok()).count();
                self.reload(false);
                self.snack(&format!("{n} item{} deleted forever", if n == 1 { "" } else { "s" }), false);
            }
            "empty-trash" => {
                let n = crate::ops::empty_trash().unwrap_or(0);
                if self.st.borrow().loc == Loc::Trash {
                    self.reload(false);
                }
                self.snack(&format!("Trash emptied ({n} item{})", if n == 1 { "" } else { "s" }), false);
            }
            _ => {}
        }
    }

    fn trash(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() || self.st.borrow().loc == Loc::Trash {
            return;
        }
        let mut moved = Vec::new();
        let mut failed = None;
        for p in &paths {
            match crate::ops::trash(p) {
                Ok(dst) => moved.push(dst),
                Err(e) => failed = Some(e),
            }
        }
        self.reload(false);
        let n = moved.len();
        self.st.borrow_mut().undo = moved;
        match failed {
            Some(e) if n == 0 => self.snack(&format!("Couldn't move to trash: {e}"), false),
            _ => self.snack(&format!("{n} item{} moved to trash", if n == 1 { "" } else { "s" }), true),
        }
    }

    fn undo(&self) {
        let items = std::mem::take(&mut self.st.borrow_mut().undo);
        let restored: Vec<PathBuf> = items.iter().filter_map(|p| crate::ops::restore(p).ok()).collect();
        self.app.set_snack_shown(false);
        self.reload(false);
        self.select_paths(&restored);
    }

    fn paste(&self, dir: PathBuf) {
        let (items, cut) = {
            let st = self.st.borrow();
            (st.clipboard.clone(), st.cut)
        };
        if items.is_empty() {
            return;
        }
        if cut {
            let mut st = self.st.borrow_mut();
            st.clipboard.clear();
            st.cut = false;
        }
        self.snack(&format!("{} {} item{}…", if cut { "Moving" } else { "Copying" }, items.len(), if items.len() == 1 { "" } else { "s" }), false);
        std::thread::spawn(move || {
            let mut done = Vec::new();
            let mut err = None;
            for p in &items {
                let r = if cut { crate::ops::move_into(p, &dir) } else { crate::ops::copy_into(p, &dir) };
                match r {
                    Ok(d) => done.push(d),
                    Err(e) => err = Some(e.to_string()),
                }
            }
            let _ = slint::invoke_from_event_loop(move || {
                with(|c| {
                    c.reload(false);
                    c.select_paths(&done);
                    match err {
                        Some(e) => c.snack(&format!("Some items failed: {e}"), false),
                        None => c.snack(&format!("{} item{} {}", done.len(), if done.len() == 1 { "" } else { "s" }, if cut { "moved" } else { "copied" }), false),
                    }
                })
            });
        });
    }

    // ---------- snackbar + dialog ----------

    fn snack(&self, text: &str, undo: bool) {
        let serial = {
            let mut st = self.st.borrow_mut();
            st.snack_serial += 1;
            st.snack_serial
        };
        self.app.set_snack_text(text.into());
        self.app.set_snack_action(if undo { "Undo".into() } else { SharedString::new() });
        self.app.set_snack_shown(true);
        Timer::single_shot(Duration::from_millis(if undo { 6000 } else { 3500 }), move || {
            with(|c| {
                if c.st.borrow().snack_serial == serial {
                    c.app.set_snack_shown(false);
                }
            })
        });
    }

    fn ask(&self, d: Dialog, title: &str, text: &str, ok: &str, select_to: usize) {
        self.st.borrow_mut().dialog = Some(d);
        self.app.set_dialog_title(title.into());
        self.app.set_dialog_text(text.into());
        self.app.set_dialog_ok(ok.into());
        self.app.set_dialog_shown(true);
        let to = select_to.min(text.chars().count()) as i32;
        // Byte offsets for the selection.
        let bytes = text.char_indices().nth(to as usize).map(|(b, _)| b).unwrap_or(text.len()) as i32;
        self.app.invoke_open_dialog(bytes);
    }

    fn dialog_close(&self) {
        self.st.borrow_mut().dialog = None;
        self.app.set_dialog_shown(false);
        self.app.invoke_focus_main();
    }

    fn dialog_accept(&self, text: &str) {
        let name = text.trim();
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            return;
        }
        let Some(d) = self.st.borrow_mut().dialog.take() else { return };
        let result: Result<PathBuf, String> = match d {
            Dialog::Rename(p) => {
                let dst = p.with_file_name(name);
                if dst == p {
                    Ok(p)
                } else if dst.exists() {
                    Err(format!("“{name}” already exists"))
                } else {
                    std::fs::rename(&p, &dst).map(|_| dst).map_err(|e| e.to_string())
                }
            }
            Dialog::NewFolder(dir) => {
                let dst = crate::ops::unique_path(&dir, name);
                std::fs::create_dir(&dst).map(|_| dst).map_err(|e| e.to_string())
            }
            Dialog::NewFile(dir) => {
                let dst = crate::ops::unique_path(&dir, name);
                std::fs::File::create_new(&dst).map(|_| dst).map_err(|e| e.to_string())
            }
        };
        self.app.set_dialog_shown(false);
        self.app.invoke_focus_main();
        match result {
            Ok(p) => {
                self.reload(false);
                self.select_paths(&[p]);
                self.build_side();
            }
            Err(e) => self.snack(&e, false),
        }
    }

    // ---------- keyboard ----------

    fn key(&self, k: &str, ctrl: bool, shift: bool, alt: bool) -> bool {
        if self.app.get_dialog_shown() {
            return false;
        }
        if alt {
            match k {
                "Left" => self.history(true),
                "Right" => self.history(false),
                "Up" => self.up(),
                _ => return false,
            }
            return true;
        }
        if ctrl {
            match k.to_lowercase().as_str() {
                "a" => self.action("select-all"),
                "c" => self.action("copy"),
                "x" => self.action("cut"),
                "v" => self.action("paste"),
                "h" => self.toggle_hidden(),
                "f" | "l" => self.app.invoke_focus_search(),
                "n" if shift => self.action("new-folder"),
                "i" => self.toggle_details(),
                "1" => self.set_view(0),
                "2" => self.set_view(1),
                "r" => self.reload(true),
                _ => return false,
            }
            return true;
        }
        match k {
            "Up" | "Down" | "Left" | "Right" | "Home" | "End" => self.move_cursor(k, shift),
            "Return" => self.open_selection(),
            "Backspace" => self.up(),
            "Delete" => {
                let p = self.selected();
                if self.st.borrow().loc == Loc::Trash { self.action("purge") } else { self.trash(p) }
            }
            "F2" => self.action("rename"),
            "F5" => self.reload(true),
            "Escape" => {
                if !self.st.borrow().search.is_empty() {
                    self.search_escape();
                } else {
                    self.press(-1, false, false);
                }
            }
            t if t.chars().count() == 1 && t.chars().all(|c| !c.is_control()) => {
                // Type to search.
                let text = format!("{}{}", self.app.get_search_text(), t);
                self.app.set_search_text(text.clone().into());
                self.search(&text);
                self.app.invoke_focus_search();
            }
            _ => return false,
        }
        true
    }

    fn move_cursor(&self, k: &str, shift: bool) {
        let cols = if self.app.get_view_mode() == 0 { self.app.get_grid_cols().max(1) as i64 } else { 1 };
        let folders = self.app.get_folder_count() as i64;
        let target = {
            let st = self.st.borrow();
            let n = st.shown.len() as i64;
            if n == 0 {
                return;
            }
            let Some(cur) = st.cursor.map(|c| c as i64) else {
                drop(st);
                self.press(0, false, false);
                self.app.invoke_reveal(0);
                return;
            };
            let grid = cols > 1;
            let t = match k {
                "Left" if grid => cur - 1,
                "Right" if grid => cur + 1,
                "Left" | "Right" => return,
                "Home" => 0,
                "End" => n - 1,
                up_down => {
                    let dir = if up_down == "Up" { -1 } else { 1 };
                    if !grid {
                        cur + dir
                    } else {
                        // Folders and files are separate grids; keep the column when crossing.
                        let (sec_start, sec_len) = if cur < folders { (0, folders) } else { (folders, n - folders) };
                        let rel = cur - sec_start;
                        let next = rel + dir * cols;
                        if next >= 0 && next < sec_len {
                            sec_start + next
                        } else if dir > 0 && cur < folders && n > folders {
                            (folders + rel % cols).min(n - 1)
                        } else if dir < 0 && cur >= folders && folders > 0 {
                            let last_row = (folders - 1) / cols;
                            (last_row * cols + rel % cols).min(folders - 1)
                        } else {
                            cur
                        }
                    }
                }
            };
            t.clamp(0, n - 1)
        };
        self.press(target as i32, false, shift);
        self.app.invoke_reveal(target as i32);
    }

    // ---------- periodic ----------

    fn tick(&self) {
        let ticks = {
            let mut st = self.st.borrow_mut();
            st.ticks += 1;
            st.ticks
        };
        let theme_now = stamp(&theme::path());
        if theme_now != self.st.borrow().theme_stamp {
            self.st.borrow_mut().theme_stamp = theme_now;
            theme::apply(&self.app);
            self.push_settings();
            self.restyle();
            self.build_side();
        }
        let watch = match &self.st.borrow().loc {
            Loc::Recent => stamp(&model::home().join(".local/share/recently-used.xbel")),
            other => other.dir().and_then(|d| stamp(&d)),
        };
        if watch != self.st.borrow().watch_stamp && !self.app.get_dialog_shown() {
            self.reload(true);
        }
        if ticks % 15 == 0 {
            self.refresh_storage();
        }
    }

    fn refresh_storage(&self) {
        let (text, frac) = storage();
        self.app.set_storage_text(text.into());
        self.app.set_storage_fraction(frac);
    }
}

fn side_match(st: &State) -> Option<(String, f32)> {
    let want = match &st.loc {
        Loc::Recent => return st.side.iter().find(|m| m.key == "recent").map(|m| (m.key.clone(), m.y)),
        Loc::Trash => return st.side.iter().find(|m| m.key == "trash").map(|m| (m.key.clone(), m.y)),
        Loc::Dir(p) => p,
    };
    st.side.iter().find(|m| m.path.as_ref() == Some(want)).map(|m| (m.key.clone(), m.y))
}

/// Breadcrumb trail: (label, path to navigate to).
fn crumbs(loc: &Loc) -> Vec<(String, Option<PathBuf>)> {
    match loc {
        Loc::Recent => vec![("Recent".into(), None)],
        Loc::Trash => vec![("Trash".into(), None)],
        Loc::Dir(p) => {
            let home = model::home();
            let (mut out, base, rest) = match p.strip_prefix(&home) {
                Ok(rest) => (vec![("Home".to_string(), Some(home.clone()))], home.clone(), rest.to_path_buf()),
                Err(_) => (vec![("File system".to_string(), Some(PathBuf::from("/")))], PathBuf::from("/"), p.strip_prefix("/").unwrap_or(p).to_path_buf()),
            };
            let mut cur = base;
            for c in rest.components() {
                cur.push(c);
                out.push((c.as_os_str().to_string_lossy().into_owned(), Some(cur.clone())));
            }
            out
        }
    }
}

fn sort_id(k: SortKey) -> &'static str {
    match k {
        SortKey::Name => "name",
        SortKey::Modified => "modified",
        SortKey::Size => "size",
        SortKey::Kind => "kind",
    }
}

fn sort_label(k: SortKey) -> &'static str {
    match k {
        SortKey::Name => "Name",
        SortKey::Modified => "Date modified",
        SortKey::Size => "Size",
        SortKey::Kind => "Kind",
    }
}

fn sort_from(s: &str) -> SortKey {
    match s {
        "modified" => SortKey::Modified,
        "size" => SortKey::Size,
        "kind" => SortKey::Kind,
        _ => SortKey::Name,
    }
}

fn load_settings(st: &mut State) {
    let Ok(text) = std::fs::read_to_string(settings_path()) else { return };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    // Older GTK builds stored "grid": bool / "info": bool.
    st.view = v["view"].as_i64().map(|x| x as i32).unwrap_or(if v["grid"].as_bool() == Some(false) { 1 } else { 0 });
    st.details = v["details"].as_bool().or(v["info"].as_bool()).unwrap_or(false);
    st.sort = sort_from(v["sort"].as_str().unwrap_or("name"));
    st.asc = v["asc"].as_bool().unwrap_or(true);
    st.hidden = v["hidden"].as_bool().unwrap_or(false);
    st.thumbnails = v["thumbnails"].as_bool().unwrap_or(true);
    st.ui_filled = v["ui_filled_icons"].as_bool().unwrap_or(false);
    st.ui_glyphs = v["ui_folder_glyphs"].as_bool().unwrap_or(false);
    st.ui_tiles = v["ui_tiles"].as_bool().unwrap_or(false);
    st.home_open = v["home_open"].as_bool().unwrap_or(true);
    let paths = |k: &str| -> Vec<PathBuf> { v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(PathBuf::from).collect()).unwrap_or_default() };
    st.expanded = paths("expanded").into_iter().collect();
    st.pins = paths("pins");
    if let Some(m) = v["colors"].as_object() {
        for (k, x) in m {
            if let Some(i) = x.as_u64() {
                st.colors.insert(k.clone(), i as usize);
            }
        }
    }
}
