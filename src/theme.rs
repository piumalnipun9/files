//! Colours: the DankMaterialShell / matugen palette mapped onto the `Pal` global,
//! plus the small fixed palettes used for folder colours and file-type tints.

use crate::model::Cat;
use crate::{AppWindow, Pal};
use slint::{Color, ComponentHandle};
use std::path::PathBuf;

pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("FILES_THEME") {
        return p.into();
    }
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| crate::model::home().join(".cache"));
    cache.join("DankMaterialShell/dms-colors.json")
}

fn hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() < 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some(Color::from_rgb_u8(p(0)?, p(2)?, p(4)?))
}

/// Push the palette into the UI. Missing file or roles keep the built-in defaults. Returns dark-mode.
pub fn apply(app: &AppWindow) -> bool {
    let pal = app.global::<Pal>();
    let Ok(text) = std::fs::read_to_string(path()) else { return pal.get_dark() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return pal.get_dark() };
    let mode = if v["mode"].as_str() == Some("light") { "light" } else { "dark" };
    pal.set_dark(mode == "dark");
    let roles = &v["colors"][mode];
    let get = |k: &str| roles[k].as_str().and_then(hex);
    macro_rules! roles {
        ($($key:literal => $set:ident),* $(,)?) => {
            $( if let Some(c) = get($key) { pal.$set(c); } )*
        };
    }
    roles! {
        "surface" => set_surface,
        "surface_container_lowest" => set_surface_container_lowest,
        "surface_container_low" => set_surface_container_low,
        "surface_container" => set_surface_container,
        "surface_container_high" => set_surface_container_high,
        "surface_container_highest" => set_surface_container_highest,
        "on_surface" => set_on_surface,
        "on_surface_variant" => set_on_surface_variant,
        "primary" => set_primary,
        "on_primary" => set_on_primary,
        "primary_container" => set_primary_container,
        "on_primary_container" => set_on_primary_container,
        "secondary_container" => set_secondary_container,
        "on_secondary_container" => set_on_secondary_container,
        "tertiary" => set_tertiary,
        "outline" => set_outline,
        "outline_variant" => set_outline_variant,
        "error" => set_error,
        "inverse_surface" => set_inverse_surface,
        "inverse_on_surface" => set_inverse_on_surface,
        "inverse_primary" => set_inverse_primary,
    }
    // Dark mode: nudge default text and the neutral (folder) tone a little lighter than matugen's.
    if mode == "dark" {
        let on = mix(pal.get_on_surface(), Color::from_rgb_u8(255, 255, 255), 0.35);
        pal.set_on_surface(on);
        pal.set_on_surface_variant(mix(pal.get_on_surface_variant(), on, 0.45));
    }
    mode == "dark"
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color::from_rgb_u8(m(a.red(), b.red()), m(a.green(), b.green()), m(a.blue(), b.blue()))
}

/// Folder colours a user can pick, Finder-tag style. Index 0 is the theme's muted neutral.
pub const SWATCH_NAMES: [&str; 9] = ["Default", "Red", "Orange", "Yellow", "Green", "Teal", "Blue", "Purple", "Grey"];
const DARK: [u32; 9] = [0, 0xff8a80, 0xffb870, 0xffd866, 0x8bd88f, 0x6fd6c8, 0x8ab4f8, 0xc9a6ff, 0xb4bcc4];
const LIGHT: [u32; 9] = [0, 0xd93025, 0xe8710a, 0xd29b00, 0x1e8e3e, 0x0b8a83, 0x1a73e8, 0x8e44d9, 0x5f6368];

pub struct Tints {
    pub dark: bool,
    pub primary: Color,
    pub neutral: Color,
}

impl Tints {
    pub fn from_app(app: &AppWindow) -> Self {
        let pal = app.global::<Pal>();
        Tints { dark: pal.get_dark(), primary: pal.get_primary(), neutral: pal.get_on_surface_variant() }
    }

    pub fn swatch(&self, i: usize) -> Color {
        let table = if self.dark { &DARK } else { &LIGHT };
        match table.get(i) {
            Some(&v) if i > 0 => Color::from_argb_encoded(0xff00_0000 | v),
            // Drive-like neutral folders unless a colour was picked.
            _ => self.neutral,
        }
    }

    pub fn swatches(&self) -> Vec<Color> {
        (0..SWATCH_NAMES.len()).map(|i| self.swatch(i)).collect()
    }

    /// Drive-like colour coding for file types.
    pub fn for_cat(&self, cat: Cat) -> Color {
        match cat {
            Cat::Pdf | Cat::Image | Cat::Video => self.swatch(1),
            Cat::Slides => self.swatch(3),
            Cat::Sheet => self.swatch(4),
            Cat::Code => self.swatch(5),
            Cat::Doc | Cat::Text => self.swatch(6),
            Cat::Audio | Cat::Font => self.swatch(7),
            Cat::Archive | Cat::Disk | Cat::Exec => self.swatch(2),
            Cat::Folder => self.neutral,
            Cat::Generic => self.neutral,
        }
    }
}
