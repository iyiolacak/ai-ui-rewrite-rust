//! User settings of the example: text size, reading width and theme, kept in a
//! small JSON file in the platform config directory. Reading and writing never
//! fail the app: a missing, unreadable or malformed file means defaults, and a
//! failed write is ignored.

use crate::{AppWindow, AssistantColorScheme, AssistantTheme};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Text scale steps of the popover and of Ctrl+= / Ctrl+-.
pub const SCALES: [f32; 4] = [0.9, 1.0, 1.15, 1.3];
pub const MIN_SCALE: f32 = 0.75;
pub const MAX_SCALE: f32 = 2.0;
/// Default text size step (`SCALES[2]`, large).
pub const DEFAULT_STEP: usize = 2;
/// Message column maximum widths at text scale 1.0: compact, default, wide,
/// extra wide, and full (the whole main area, `f32::INFINITY` here).
pub const WIDTHS: [f32; 5] = [576.0, 704.0, 880.0, 1100.0, f32::INFINITY];
/// Names of [`WIDTHS`] in the settings file and `--reading-width`.
pub const WIDTH_NAMES: [&str; 5] = ["compact", "default", "wide", "xwide", "full"];
/// Index of the full-width option.
pub const FULL_WIDTH: usize = 4;
/// What the window is given for "full"; the layout clamps it to the area.
const FULL_PX: f32 = 100_000.0;
/// Side padding of the column: comfortable for full width, upstream's otherwise.
pub const FULL_PADDING: f32 = 40.0;
pub const PADDING: f32 = 16.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn index(self) -> i32 {
        self as i32
    }
    pub fn from_index(index: i32) -> Self {
        match index {
            0 => Theme::System,
            1 => Theme::Light,
            _ => Theme::Dark,
        }
    }
    fn name(self) -> &'static str {
        ["system", "light", "dark"][self as usize]
    }
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "system" => Some(Theme::System),
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Settings {
    pub text_scale: f32,
    /// Index into [`WIDTHS`].
    pub width: usize,
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            text_scale: SCALES[DEFAULT_STEP],
            width: 1,
            // The example has always started dark.
            theme: Theme::Dark,
        }
    }
}

pub fn clamp_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(MIN_SCALE, MAX_SCALE)
    } else {
        SCALES[DEFAULT_STEP]
    }
}

impl Settings {
    /// Index of the step nearest to the current scale.
    pub fn text_step(&self) -> usize {
        let mut best = 0;
        for (i, s) in SCALES.iter().enumerate() {
            if (s - self.text_scale).abs() < (SCALES[best] - self.text_scale).abs() {
                best = i;
            }
        }
        best
    }

    pub fn set_step(&mut self, step: usize) {
        self.text_scale = SCALES[step.min(SCALES.len() - 1)];
    }

    /// Ctrl+= (+1), Ctrl+- (-1), Ctrl+0 (0: reset). Stops at the ends.
    pub fn adjust(&mut self, delta: i32) {
        if delta == 0 {
            self.set_step(DEFAULT_STEP);
            return;
        }
        // A scale between steps (from `--text-scale`) moves to the next step
        // in the requested direction rather than the nearest one.
        let current = self.text_scale;
        let next = if delta > 0 {
            SCALES.iter().position(|s| *s > current + 1e-3)
        } else {
            SCALES.iter().rposition(|s| *s < current - 1e-3)
        };
        if let Some(step) = next {
            self.set_step(step);
        }
    }

    pub fn set_width(&mut self, index: usize) {
        self.width = index.min(WIDTHS.len() - 1);
    }

    /// Column width in logical px: the option's width times the text scale, so
    /// the line length in characters stays the same; the layout never lets it
    /// exceed the available area. Full is the whole area.
    pub fn column_width(&self) -> f32 {
        let width = WIDTHS[self.width.min(WIDTHS.len() - 1)];
        if width.is_finite() {
            width * self.text_scale
        } else {
            FULL_PX
        }
    }

    pub fn column_padding(&self) -> f32 {
        if self.width == FULL_WIDTH {
            FULL_PADDING
        } else {
            PADDING
        }
    }

    pub fn to_json(self) -> String {
        format!(
            "{{\n  \"text_scale\": {},\n  \"reading_width\": \"{}\",\n  \"theme\": \"{}\"\n}}\n",
            self.text_scale,
            WIDTH_NAMES[self.width.min(WIDTH_NAMES.len() - 1)],
            self.theme.name()
        )
    }

    /// Unknown or invalid fields keep their defaults; numbers are clamped.
    pub fn from_json(text: &str) -> Self {
        let mut settings = Self::default();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return settings;
        };
        if let Some(scale) = value.get("text_scale").and_then(|v| v.as_f64()) {
            settings.text_scale = clamp_scale(scale as f32);
        }
        if let Some(width) = value.get("reading_width").and_then(|v| v.as_str()) {
            if let Some(i) = WIDTH_NAMES.iter().position(|n| *n == width) {
                settings.width = i;
            }
        }
        if let Some(theme) = value
            .get("theme")
            .and_then(|v| v.as_str())
            .and_then(Theme::parse)
        {
            settings.theme = theme;
        }
        settings
    }
}

/// `$XDG_CONFIG_HOME` or `~/.config` (`%APPDATA%` on Windows), then the app's folder.
pub fn config_path() -> Option<PathBuf> {
    let base = non_empty_var("XDG_CONFIG_HOME")
        .or_else(|| non_empty_var("HOME").map(|home| home.join(".config")))
        .or_else(|| non_empty_var("APPDATA"))?;
    Some(base.join("ai-ui-example-thread").join("settings.json"))
}

fn non_empty_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub fn load_from(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .map(|text| Settings::from_json(&text))
        .unwrap_or_default()
}

/// Write through a temporary file in the same folder, then rename it over the
/// target, so a crash leaves the old file or the new one. Errors are ignored.
pub fn save_to(path: &Path, settings: &Settings) {
    let _ = try_save(path, settings);
}

fn try_save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".settings.{}.tmp", std::process::id()));
    std::fs::write(&tmp, settings.to_json())?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Push the settings into the window: one global (`text-scale`) read by every
/// font size of the message text, Markdown, code and composer, plus the
/// column width and theme. Nothing is re-parsed or re-formatted: Markdown
/// blocks carry no sizes, and only the rows on screen re-evaluate.
pub fn apply(ui: &AppWindow, settings: &Settings) {
    let theme = ui.global::<AssistantTheme>();
    theme.set_text_scale(settings.text_scale);
    theme.set_follow_system(settings.theme == Theme::System);
    theme.set_scheme(if settings.theme == Theme::Light {
        AssistantColorScheme::Light
    } else {
        AssistantColorScheme::Dark
    });
    // The window's logical width drives the compact breakpoint. The offscreen
    // renderer reports it as the physical size divided by its 2x scale factor,
    // so write it here as well as from `changed width` in the markup (which
    // handles resizes after `show`, and settings changes re-apply it too).
    let physical = ui.window().size();
    let scale = ui.window().scale_factor();
    theme.set_viewport_width(physical.width as f32 / scale);
    ui.set_text_size_index(settings.text_step() as i32);
    ui.set_reading_width_index(settings.width as i32);
    ui.set_theme_index(settings.theme.index());
    ui.set_column_width(settings.column_width());
    ui.set_column_padding(settings.column_padding());
}

/// Apply `initial`, and handle the window's settings callbacks: every change is
/// applied and, when `path` is given, saved.
pub fn install(ui: &AppWindow, initial: Settings, path: Option<PathBuf>) -> Rc<RefCell<Settings>> {
    apply(ui, &initial);
    let state = Rc::new(RefCell::new(initial));
    let change = {
        let (state, weak) = (state.clone(), ui.as_weak());
        Rc::new(move |edit: &dyn Fn(&mut Settings)| {
            let mut settings = state.borrow_mut();
            let before = *settings;
            edit(&mut settings);
            if *settings == before {
                return;
            }
            if let Some(ui) = weak.upgrade() {
                apply(&ui, &settings);
            }
            if let Some(path) = &path {
                save_to(path, &settings);
            }
        })
    };
    let c = change.clone();
    ui.on_text_size_selected(move |i| c(&|s| s.set_step(i.max(0) as usize)));
    let c = change.clone();
    ui.on_reading_width_selected(move |i| c(&|s| s.set_width(i.max(0) as usize)));
    let c = change.clone();
    ui.on_theme_selected(move |i| c(&|s| s.theme = Theme::from_index(i)));
    ui.on_adjust_text_size(move |delta| change(&|s| s.adjust(delta)));
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ai-ui-settings-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn settings_round_trip_through_a_file_created_on_demand() {
        let dir = dir("round-trip");
        let path = dir.join("nested").join("settings.json");
        let saved = Settings {
            text_scale: 1.15,
            width: 2,
            theme: Theme::Light,
        };
        save_to(&path, &saved);
        assert_eq!(load_from(&path), saved);
        // No temporary file is left behind.
        let leftovers = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_or_malformed_files_give_defaults_and_bad_fields_are_ignored() {
        let dir = dir("malformed");
        assert_eq!(load_from(&dir.join("absent.json")), Settings::default());
        assert_eq!(Settings::from_json("not json"), Settings::default());
        assert_eq!(Settings::from_json("[1]"), Settings::default());
        let partly =
            Settings::from_json(r#"{"text_scale":"big","reading_width":"wide","theme":"neon"}"#);
        assert_eq!(
            partly,
            Settings {
                width: 2,
                ..Settings::default()
            }
        );
        // Saving into an unwritable place (a file as parent) does not panic.
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        save_to(&blocker.join("settings.json"), &Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_scale_is_clamped_on_load_and_steps_stop_at_the_ends() {
        assert_eq!(
            Settings::from_json(r#"{"text_scale":9}"#).text_scale,
            MAX_SCALE
        );
        assert_eq!(
            Settings::from_json(r#"{"text_scale":0.1}"#).text_scale,
            MIN_SCALE
        );
        assert_eq!(clamp_scale(f32::NAN), SCALES[DEFAULT_STEP]);
        let mut s = Settings::default();
        s.adjust(-1);
        s.adjust(-1);
        assert_eq!(s.text_scale, 0.9);
        for _ in 0..6 {
            s.adjust(1);
        }
        assert_eq!(s.text_scale, 1.3);
        s.adjust(0);
        assert_eq!(s.text_scale, SCALES[DEFAULT_STEP]);
        // Between steps, a keypress moves to the next step in its direction.
        s.text_scale = 1.05;
        s.adjust(1);
        assert_eq!(s.text_scale, 1.15);
        s.text_scale = 1.05;
        s.adjust(-1);
        assert_eq!(s.text_scale, 1.0);
        s.text_scale = 1.9;
        assert_eq!(s.text_step(), 3);
    }

    #[test]
    fn theme_and_width_indices_match_the_popover_lists() {
        assert_eq!(Theme::from_index(0), Theme::System);
        assert_eq!(Theme::from_index(1), Theme::Light);
        assert_eq!(Theme::from_index(2), Theme::Dark);
        assert_eq!(Theme::Dark.index(), 2);
        let mut s = Settings::default();
        s.set_width(99);
        assert_eq!(s.width, FULL_WIDTH);
        assert_eq!(s.column_width(), FULL_PX);
        assert_eq!(s.column_padding(), FULL_PADDING);
        s.set_width(2);
        assert_eq!(s.column_width(), 880.0 * s.text_scale);
        s.set_width(0);
        assert_eq!(s.column_width(), 576.0 * s.text_scale);
        assert_eq!(s.column_padding(), PADDING);
    }

    #[test]
    fn column_width_follows_the_text_scale_and_old_values_still_load() {
        let mut s = Settings {
            text_scale: 1.3,
            ..Settings::default()
        };
        assert!((s.column_width() - 915.2).abs() < 0.01);
        s.set_width(3);
        assert!((s.column_width() - 1430.0).abs() < 0.01);
        for (i, name) in WIDTH_NAMES.iter().enumerate() {
            let json = format!(r#"{{"reading_width":"{name}"}}"#);
            assert_eq!(Settings::from_json(&json).width, i);
        }
        // A file written before the two new options.
        let old =
            Settings::from_json(r#"{"text_scale":1.15,"reading_width":"wide","theme":"light"}"#);
        assert_eq!(
            (old.text_scale, old.width, old.theme),
            (1.15, 2, Theme::Light)
        );
        let round = Settings::from_json(&Settings { width: 3, ..old }.to_json());
        assert_eq!(round.width, 3);
    }
}
