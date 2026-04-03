use chrono::{DateTime, TimeDelta, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::theme::Theme;

/// A toast notification that displays temporarily in the bottom-right corner.
#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub expires: DateTime<Utc>,
}

impl Toast {
    /// Create a new toast with the given message and duration in seconds.
    pub fn new(message: String, duration_secs: i64) -> Self {
        Self {
            message,
            expires: Utc::now() + TimeDelta::seconds(duration_secs),
        }
    }

    /// Returns true if the toast has expired and should be removed.
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires
    }
}

/// Renders a toast notification in the bottom-right corner of the area.
/// No-op if the toast is None.
///
/// `area` should typically be the full terminal area (frame.area()).
pub fn draw_toast(frame: &mut Frame, toast: &Toast, theme: &Theme) {
    let area = frame.area();
    let toast_width = toast.message.len() as u16 + 4; // +4 for borders and padding
    let toast_height: u16 = 3;

    let x = area.width.saturating_sub(toast_width).saturating_sub(1);
    let y = area
        .height
        .saturating_sub(toast_height)
        .saturating_sub(2); // above status bar

    let toast_area = Rect::new(x, y, toast_width.min(area.width), toast_height);

    let widget = Paragraph::new(toast.message.as_str())
        .style(theme.toast_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.toast_border),
        )
        .alignment(Alignment::Center);

    frame.render_widget(Clear, toast_area);
    frame.render_widget(widget, toast_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_not_expired_immediately() {
        let t = Toast::new("Hello".to_string(), 5);
        assert!(!t.is_expired());
    }

    #[test]
    fn toast_expired_in_past() {
        let t = Toast {
            message: "Old".to_string(),
            expires: Utc::now() - TimeDelta::seconds(1),
        };
        assert!(t.is_expired());
    }

    #[test]
    fn toast_message_preserved() {
        let t = Toast::new("Test message".to_string(), 3);
        assert_eq!(t.message, "Test message");
    }
}
