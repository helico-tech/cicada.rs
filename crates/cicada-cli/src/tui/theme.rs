//! Colours. Only the 16 ANSI colours and the terminal's own foreground, so the TUI reads on light and dark terminals.

use ratatui::style::{Color, Modifier, Style};

/// Borders, titles and the selected row.
pub const ACCENT: Color = Color::Cyan;
/// Key names in hints.
pub const KEY: Color = Color::LightBlue;
/// Labels in the header and in facts.
pub const LABEL: Color = Color::Yellow;
/// The logo.
pub const LOGO: Color = Color::LightGreen;

/// The logo, three lines.
pub const LOGO_LINES: [&str; 3] = ["╔═╗╦╔═╗╔═╗╔╦╗╔═╗", "║  ║║  ╠═╣ ║║╠═╣", "╚═╝╩╚═╝╩ ╩═╩╝╩ ╩"];

/// What a value means, for its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Nothing special.
    Normal,
    /// Background information, finished business.
    Dim,
    /// Healthy, done.
    Good,
    /// Waiting, paused, suspect.
    Warn,
    /// Failed, dead, an error.
    Bad,
    /// Busy right now.
    Info,
    /// Purged.
    Gone,
}

impl Tone {
    /// The style of text in this tone.
    pub fn style(self) -> Style {
        let fg = match self {
            Tone::Normal => return Style::new(),
            Tone::Dim => Color::DarkGray,
            Tone::Good => Color::Green,
            Tone::Warn => Color::Yellow,
            Tone::Bad => Color::Red,
            Tone::Info => Color::Cyan,
            Tone::Gone => Color::Magenta,
        };
        Style::new().fg(fg)
    }
}

/// The tone of a status word (`pending`, `failed`, `alive`, …).
pub fn status(word: &str) -> Tone {
    match word {
        "done" | "alive" | "active" => Tone::Good,
        "pending" | "suspect" | "paused" => Tone::Warn,
        "failed" | "dead" => Tone::Bad,
        "running" => Tone::Info,
        "cancelled" => Tone::Dim,
        "purged" => Tone::Gone,
        _ => Tone::Normal,
    }
}

/// True for the columns that hold a status word.
pub fn is_status_column(header: &str) -> bool {
    matches!(header, "STATUS" | "STATE")
}

/// The tone of one cell, from its column and value.
pub fn cell(header: &str, value: &str) -> Tone {
    match header {
        h if is_status_column(h) => status(value),
        "PAUSED" if value == "true" => Tone::Warn,
        "CANCEL-REQUESTED" if value == "true" => Tone::Bad,
        "ERROR" | "LAST-ERROR" if value != "-" => Tone::Bad,
        "FAILURES" if value != "0" => Tone::Warn,
        "DUE" | "LEASE" if value.starts_with("in ") => Tone::Dim,
        _ => Tone::Normal,
    }
}

/// The tone of a whole row, from its status: failures stand out, finished business fades.
pub fn row(status: Option<&str>) -> Tone {
    match status.map(self::status) {
        Some(Tone::Bad) => Tone::Bad,
        Some(Tone::Dim | Tone::Gone) => Tone::Dim,
        _ => Tone::Normal,
    }
}

/// Bold text.
pub fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_and_cells_get_their_tones() {
        assert_eq!(cell("STATUS", "failed"), Tone::Bad);
        assert_eq!(cell("STATE", "running"), Tone::Info);
        assert_eq!(cell("ERROR", "-"), Tone::Normal);
        assert_eq!(cell("ERROR", "boom"), Tone::Bad);
        assert_eq!(cell("DUE", "in 5m"), Tone::Dim);
        assert_eq!(row(Some("cancelled")), Tone::Dim);
        assert_eq!(row(Some("done")), Tone::Normal);
        assert_eq!(row(None), Tone::Normal);
    }
}
