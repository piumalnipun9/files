mod ctl;
mod model;
mod ops;
mod theme;
mod thumbs;

slint::include_modules!();

use slint::{ComponentHandle, Timer};
use std::path::PathBuf;
use std::time::Duration;

fn main() -> Result<(), slint::PlatformError> {
    // The .desktop entry passes %U, so accept file:// URIs as well as paths.
    let start = std::env::args()
        .nth(1)
        .map(|a| a.strip_prefix("file://").map(ops::percent_decode).unwrap_or(a))
        .map(PathBuf::from)
        .and_then(|p| std::path::absolute(p).ok());
    let app = AppWindow::new()?;
    let c = ctl::Ctl::install(app.clone_strong());
    c.go(ctl::Loc::Dir(start.filter(|p| p.is_dir()).unwrap_or_else(model::home)));
    app.invoke_focus_main();
    if let Some(out) = std::env::var_os("FILES_SHOT") {
        shot(&app, PathBuf::from(out));
    }
    app.run()
}

/// Developer aid: `FILES_SHOT=out.png [FILES_ACT=a,b,…] files [dir]` renders the window to a PNG and quits.
/// Actions are menu ids, plus `list`, `grid`, `select:N`, `ctx:N`, `new`, `key:K`.
fn shot(app: &AppWindow, out: PathBuf) {
    let acts: Vec<String> = std::env::var("FILES_ACT").map(|s| s.split(',').map(String::from).collect()).unwrap_or_default();
    for (n, act) in acts.into_iter().enumerate() {
        Timer::single_shot(Duration::from_millis(500 + 350 * n as u64), move || {
            ctl::with(|c| match act.split_once(':') {
                Some(("select", i)) => c.app.invoke_press(i.parse().unwrap_or(0), true, false),
                Some(("ctx", i)) => c.app.invoke_context(i.parse().unwrap_or(0), 600.0, 300.0),
                Some(("key", k)) => {
                    c.app.invoke_key(k.into(), false, false, false);
                }
                Some(("side", k)) => c.app.invoke_side_toggle(k.into()),
                _ if act == "list" => c.app.invoke_set_view(1),
                _ if act == "grid" => c.app.invoke_set_view(0),
                _ if act == "new" => c.app.invoke_new_menu(12.0, 70.0),
                _ => c.action(&act),
            });
        });
    }
    let delay = std::env::var("FILES_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
    let weak = app.as_weak();
    Timer::single_shot(Duration::from_millis(delay), move || {
        let Some(app) = weak.upgrade() else { return };
        match app.window().take_snapshot() {
            Ok(px) => {
                if let Some(img) = image::RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()) {
                    let _ = img.save(&out);
                }
            }
            Err(e) => eprintln!("snapshot failed: {e}"),
        }
        let _ = slint::quit_event_loop();
    });
}
