//! Presentation helpers layered on cliclack/console: per-category theming so a
//! whole suggestion card is colored by what it optimizes, plus status glyphs and
//! path tidying shared by the report and the wizard.

use crate::suggestion::Tag;
use cliclack::{Theme, ThemeState};
use console::{Style, StyledObject, style};
use std::path::Path;

/// Accent color per category: disk is the scarce-resource warning hue, speed the
/// cool "fast" hue, both the green win-win. Reused for the gutter, glyphs, and the
/// live job tail so a card reads as one unit.
pub fn accent(tag: Tag) -> Style {
    match tag {
        Tag::Disk => Style::new().yellow(),
        Tag::Speed => Style::new().cyan(),
        Tag::Both => Style::new().green(),
    }
}

/// A cliclack theme that paints the vertical gutter and step glyph in one
/// category color. Errors and cancels stay red so failures remain obvious.
struct CategoryTheme {
    color: Style,
}

impl Theme for CategoryTheme {
    fn bar_color(&self, state: &ThemeState) -> Style {
        match state {
            ThemeState::Cancel | ThemeState::Error(_) => Style::new().red(),
            _ => self.color.clone(),
        }
    }

    fn state_symbol_color(&self, state: &ThemeState) -> Style {
        self.bar_color(state)
    }
}

/// Tint the global cliclack theme for the next card's category.
pub fn set_category(tag: Tag) {
    cliclack::set_theme(CategoryTheme { color: accent(tag) });
}

/// Restore the default cliclack look for framing, the report, and the summary.
pub fn reset() {
    cliclack::reset_theme();
}

/// Color a disk-usage percentage by how alarming it is.
pub fn disk_style(used_pct: u64) -> Style {
    match used_pct {
        90..=u64::MAX => Style::new().red().bold(),
        75..=89 => Style::new().yellow(),
        _ => Style::new().dim(),
    }
}

pub fn check() -> StyledObject<&'static str> {
    style("✓").green()
}

pub fn cross() -> StyledObject<&'static str> {
    style("✗").red().dim()
}

/// Replace the home prefix with `~` for display only, so long paths stay short.
pub fn tildify(path: &Path) -> String {
    let s = path.display().to_string();
    let Some(home) = dirs::home_dir() else {
        return s;
    };
    let home = home.display().to_string();
    if !home.is_empty()
        && let Some(rest) = s.strip_prefix(&home)
    {
        return format!("~{rest}");
    }
    s
}
