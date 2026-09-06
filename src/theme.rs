//! Shared colour vocabulary for terminal output.
//!
//! Every helper degrades to plain text when `console` has colours disabled, so
//! `--no-color`, `NO_COLOR` and piped output keep the exact same wording.

use console::{Style, StyledObject, style};
use std::fmt::Display;
use std::path::Path;

const BRAND: u8 = 117;
const ACCENT: u8 = 141;
const GOOD: u8 = 114;
const WARN: u8 = 221;
const BAD: u8 = 210;
const MUTED: u8 = 245;

/// Section heading, e.g. `▌ PLAN`.
pub(crate) fn heading(text: &str) -> String {
    format!(
        "{} {}",
        Style::new().color256(ACCENT).apply_to("▌"),
        Style::new().color256(ACCENT).bold().apply_to(text)
    )
}

/// Product banner used at the top of the status dashboard.
pub(crate) fn banner(tagline: &str) -> String {
    format!(
        "{} {}  {}",
        Style::new().color256(BRAND).bold().apply_to("◆"),
        Style::new().white().bold().apply_to("skill-issue"),
        dim(tagline)
    )
}

pub(crate) fn ok() -> StyledObject<&'static str> {
    Style::new().color256(GOOD).bold().apply_to("✓")
}

pub(crate) fn warn() -> StyledObject<&'static str> {
    Style::new().color256(WARN).bold().apply_to("⚠")
}

pub(crate) fn bad() -> StyledObject<&'static str> {
    Style::new().color256(BAD).bold().apply_to("✗")
}

fn stderr_style(colour: u8) -> Style {
    Style::new().color256(colour).bold().for_stderr()
}

pub(crate) fn error_prefix() -> String {
    stderr_style(BAD).apply_to("Error:").to_string()
}

/// `⚠` for diagnostics written to stderr.
pub(crate) fn warn_err() -> StyledObject<&'static str> {
    stderr_style(WARN).apply_to("⚠")
}

/// `✗` for diagnostics written to stderr.
pub(crate) fn bad_err() -> StyledObject<&'static str> {
    stderr_style(BAD).apply_to("✗")
}

/// A suggested command inside a stderr diagnostic.
pub(crate) fn hint_err(command: &str) -> StyledObject<String> {
    stderr_style(BRAND).apply_to(command.to_string())
}

pub(crate) fn dot() -> StyledObject<&'static str> {
    Style::new().color256(MUTED).apply_to("•")
}

pub(crate) fn arrow() -> StyledObject<&'static str> {
    Style::new().color256(MUTED).apply_to("->")
}

/// A metric worth reading first.
pub(crate) fn count(value: impl Display) -> StyledObject<String> {
    Style::new()
        .color256(BRAND)
        .bold()
        .apply_to(value.to_string())
}

pub(crate) fn good_count(value: impl Display) -> StyledObject<String> {
    Style::new()
        .color256(GOOD)
        .bold()
        .apply_to(value.to_string())
}

pub(crate) fn warn_count(value: impl Display) -> StyledObject<String> {
    Style::new()
        .color256(WARN)
        .bold()
        .apply_to(value.to_string())
}

pub(crate) fn bad_count(value: impl Display) -> StyledObject<String> {
    Style::new()
        .color256(BAD)
        .bold()
        .apply_to(value.to_string())
}

pub(crate) fn skill(name: &str) -> StyledObject<String> {
    style(name.to_string()).white().bold()
}

pub(crate) fn agent(label: &str) -> StyledObject<String> {
    Style::new().color256(BRAND).apply_to(label.to_string())
}

/// Left-aligned agent label that stays aligned once colour codes are added.
pub(crate) fn agent_padded(label: &str, width: usize) -> StyledObject<String> {
    Style::new()
        .color256(BRAND)
        .apply_to(format!("{label:<width$}"))
}

pub(crate) fn path(value: &Path) -> StyledObject<String> {
    dim_owned(value.display().to_string())
}

pub(crate) fn path_text(value: &str) -> StyledObject<String> {
    dim_owned(value.to_string())
}

pub(crate) fn fingerprint(value: &str) -> StyledObject<String> {
    Style::new().color256(ACCENT).apply_to(value.to_string())
}

pub(crate) fn dim(text: &str) -> StyledObject<String> {
    dim_owned(text.to_string())
}

fn dim_owned(text: String) -> StyledObject<String> {
    Style::new().color256(MUTED).apply_to(text)
}

/// A command the reader is expected to run next.
pub(crate) fn hint(command: &str) -> StyledObject<String> {
    Style::new()
        .color256(BRAND)
        .bold()
        .apply_to(command.to_string())
}

/// Plan verbs such as `LINK` or `REMOVE`, coloured by how destructive they are.
pub(crate) fn action(verb: &str) -> StyledObject<String> {
    let colour = match verb {
        "LINK" | "RELINK" | "CREATE" | "CREATE TARGET" | "ADD TARGET" | "ENABLE" => GOOD,
        "UNLINK" | "REMOVE" | "DISABLE" | "REMOVE TARGET" | "REMOVE IGNORE" | "STAGE" => WARN,
        "MOVE" | "COPY" | "PRESERVE" | "RECREATE LINK" => ACCENT,
        _ => BRAND,
    };
    Style::new()
        .color256(colour)
        .bold()
        .apply_to(verb.to_string())
}

/// Colour a unified diff the way Git does. Returns the input untouched when
/// colours are disabled.
pub(crate) fn diff(patch: &str) -> String {
    if !console::colors_enabled() {
        return patch.to_string();
    }
    patch
        .lines()
        .map(|line| {
            let styled = if line.starts_with("@@") {
                Style::new().color256(BRAND).apply_to(line)
            } else if line.starts_with("+++") || line.starts_with("---") {
                Style::new().color256(MUTED).bold().apply_to(line)
            } else if line.starts_with('+') {
                Style::new().color256(GOOD).apply_to(line)
            } else if line.starts_with('-') {
                Style::new().color256(BAD).apply_to(line)
            } else {
                Style::new().color256(MUTED).apply_to(line)
            };
            format!("{styled}\n")
        })
        .collect()
}

/// Compact coverage meter. Emitted only when colours are available, so plain
/// output stays free of decorative block characters.
pub(crate) fn gauge(done: usize, total: usize, width: usize) -> String {
    if !console::colors_enabled() || total == 0 || width == 0 {
        return String::new();
    }
    let filled = filled_cells(done, total, width);
    let colour = if done == total {
        GOOD
    } else if done == 0 {
        BAD
    } else {
        WARN
    };
    let mut meter = String::new();
    if filled > 0 {
        meter.push_str(
            &Style::new()
                .color256(colour)
                .apply_to("█".repeat(filled))
                .to_string(),
        );
    }
    if filled < width {
        meter.push_str(
            &Style::new()
                .color256(238)
                .apply_to("█".repeat(width - filled))
                .to_string(),
        );
    }
    meter
}

/// Any progress at all lights up at least one cell, and only completion fills
/// the meter.
fn filled_cells(done: usize, total: usize, width: usize) -> usize {
    if total == 0 {
        return 0;
    }
    (done * width).div_ceil(total).min(width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_helper_paints_when_enabled_and_stays_plain_when_disabled() {
        let previous = console::colors_enabled();

        console::set_colors_enabled(true);
        assert!(heading("PLAN").contains("\u{1b}["));
        assert!(banner("tagline").contains("\u{1b}["));
        assert!(action("LINK").to_string().contains("\u{1b}["));
        assert!(count(7).to_string().contains('7'));
        assert!(diff("+added\n-removed\n@@ hunk @@\n").contains("\u{1b}["));
        assert!(gauge(1, 2, 8).contains('█'));

        console::set_colors_enabled(false);
        assert_eq!(heading("PLAN"), "▌ PLAN");
        assert_eq!(banner("tagline"), "◆ skill-issue  tagline");
        assert_eq!(action("LINK").to_string(), "LINK");
        assert_eq!(count(7).to_string(), "7");
        assert_eq!(diff("+added\n"), "+added\n");
        assert!(gauge(1, 2, 8).is_empty());

        console::set_colors_enabled(previous);
    }

    #[test]
    fn the_gauge_only_fills_on_complete_coverage() {
        assert_eq!(filled_cells(0, 3, 12), 0);
        assert_eq!(filled_cells(1, 100, 12), 1);
        assert_eq!(filled_cells(2, 3, 12), 8);
        assert_eq!(filled_cells(3, 3, 12), 12);
        assert_eq!(filled_cells(1, 0, 12), 0);
    }
}
