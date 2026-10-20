//! Project status in the tray: a colored dot on the icon and a short tooltip.
use crate::runtime::{Core, Snapshot};
use std::sync::Mutex;
use tauri::Manager;

pub const TRAY_ID: &str = "main";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    None,
    /// Teammates' updates are ready to apply.
    Updates,
    /// Something needs attention: a conflict, an unfinished install, or offline.
    Attention,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn summary(s: &Snapshot) -> (Badge, String) {
    let Some(project) = &s.project else {
        return (Badge::None, "miliastra/collab".into());
    };
    let mut badge = Badge::None;
    let mut lines = vec![];
    if !s.online {
        badge = Badge::Attention;
        lines.push("Offline: your changes stay on this PC".to_string());
    }
    if s.installation.as_ref().is_some_and(|i| i.pending()) {
        badge = Badge::Attention;
        lines.push("Finish applying updates".to_string());
    }
    let conflicts = s.sections.iter().filter(|x| x.conflict).count();
    if conflicts > 0 {
        badge = Badge::Attention;
        lines.push(plural(
            conflicts,
            "conflict to review",
            "conflicts to review",
        ));
    }
    let incoming: Vec<_> = s
        .sections
        .iter()
        .filter(|x| !x.installed && !x.conflict && !x.installation_pending)
        .collect();
    let ready = incoming.iter().filter(|x| x.downloaded).count();
    if ready > 0 {
        if badge == Badge::None {
            badge = Badge::Updates;
        }
        lines.push(plural(
            ready,
            "update ready to apply",
            "updates ready to apply",
        ));
    } else if !incoming.is_empty() {
        lines.push(plural(
            incoming.len(),
            "update downloading",
            "updates downloading",
        ));
    }
    let unshared = s
        .sections
        .iter()
        .filter(|x| x.changed && !x.conflict)
        .count();
    if unshared > 0 {
        lines.push(plural(
            unshared,
            "slice not shared yet",
            "slices not shared yet",
        ));
    }
    if lines.is_empty() {
        lines.push("Up to date".into());
    }
    let mut text = format!("{}\n{}", project.name, lines.join("\n"));
    // Windows truncates tray tooltips at 127 characters.
    if text.chars().count() > 120 {
        text = text.chars().take(119).collect::<String>() + "…";
    }
    (badge, text)
}

/// Draw a status dot (with a white ring) in the bottom-right corner of an RGBA icon.
pub fn badge_icon(rgba: &[u8], width: u32, height: u32, badge: Badge) -> Vec<u8> {
    let mut out = rgba.to_vec();
    let color: [u8; 3] = match badge {
        Badge::None => return out,
        Badge::Updates => [0x2b, 0x7d, 0xe9],
        Badge::Attention => [0xd9, 0x30, 0x25],
    };
    let size = width.min(height) as f32;
    let radius = size * 0.24;
    let ring = (size * 0.06).max(1.0);
    let (cx, cy) = (width as f32 - radius - 1.0, height as f32 - radius - 1.0);
    for y in 0..height {
        for x in 0..width {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let i = ((y * width + x) * 4) as usize;
            if d <= radius - ring {
                out[i..i + 3].copy_from_slice(&color);
                out[i + 3] = 255;
            } else if d <= radius {
                out[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    out
}

static SHOWN: Mutex<Option<(Badge, String)>> = Mutex::new(None);

pub fn refresh(app: &tauri::AppHandle) {
    let Ok(snapshot) = app.state::<Core>().snapshot() else {
        return;
    };
    let (badge, tooltip) = summary(&snapshot);
    let mut shown = SHOWN.lock().unwrap();
    if shown.as_ref() == Some(&(badge, tooltip.clone())) {
        return;
    }
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let _ = tray.set_tooltip(Some(&tooltip));
    if shown.as_ref().is_none_or(|(b, _)| *b != badge) {
        if let Some(icon) = app.default_window_icon() {
            let rgba = badge_icon(icon.rgba(), icon.width(), icon.height(), badge);
            let _ = tray.set_icon(Some(tauri::image::Image::new_owned(
                rgba,
                icon.width(),
                icon.height(),
            )));
        }
    }
    *shown = Some((badge, tooltip));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_dot_colors_only_the_corner_and_leaves_the_icon_unchanged_without_status() {
        let (w, h) = (32, 32);
        let plain = vec![10u8; (w * h * 4) as usize];
        assert_eq!(badge_icon(&plain, w, h, Badge::None), plain);
        let dotted = badge_icon(&plain, w, h, Badge::Attention);
        let pixel = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            dotted[i..i + 4].to_vec()
        };
        assert_eq!(pixel(0, 0), vec![10, 10, 10, 10], "top-left untouched");
        assert_eq!(pixel(w - 8, h - 8), vec![0xd9, 0x30, 0x25, 255], "red dot");
        assert_eq!(
            &badge_icon(&plain, w, h, Badge::Updates)[((h - 8) * w + (w - 8)) as usize * 4..][..3],
            &[0x2b, 0x7d, 0xe9]
        );
    }
    #[test]
    fn plural_wording() {
        assert_eq!(plural(1, "update", "updates"), "1 update");
        assert_eq!(plural(3, "update", "updates"), "3 updates");
    }
}
