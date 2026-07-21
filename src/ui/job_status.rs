//! Always-visible, non-modal status area for the confirmed-action job queue
//! (see [`crate::job_queue`]). Occupies 2 rows above the status bar while a
//! job is running, queued, or a completion summary is lingering; otherwise
//! `ui::render::draw` collapses the layout so this area takes no space.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::job_queue::JobStatusView;
use crate::theme::Theme;

use super::shared::render_progress_bar;

const BAR_WIDTH: usize = 24;

pub fn render_job_status(frame: &mut Frame, area: Rect, view: &JobStatusView, theme: &Theme) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let line1 = current_line(view);
    frame.render_widget(Paragraph::new(line1).style(theme.primary_text), rows[0]);

    let line2 = queue_line(view);
    frame.render_widget(Paragraph::new(line2).style(theme.dim), rows[1]);
}

/// Line 1: the running job's own progress, or -- once idle -- the lingering
/// completion summary from the job that just finished.
fn current_line(view: &JobStatusView) -> String {
    if let Some(label) = view.current_label {
        match view.current_progress {
            Some(progress) => {
                let bar = render_progress_bar(BAR_WIDTH, progress.completed, progress.total);
                format!(" {label}: {bar} {}", progress.current_item)
            }
            None => format!(" {label}..."),
        }
    } else if let Some(summary) = view.summary {
        if summary.fail_count > 0 {
            format!(
                " {}: {} succeeded, {} failed",
                summary.label, summary.success_count, summary.fail_count
            )
        } else {
            format!(" {}: {} succeeded", summary.label, summary.success_count)
        }
    } else {
        String::new()
    }
}

/// Line 2: aggregate progress across the whole queue. Blank once nothing is
/// running or queued (the aggregate bar isn't meaningful during the
/// lingering-summary tail).
fn queue_line(view: &JobStatusView) -> String {
    if view.current_label.is_none() && view.queued_count == 0 {
        return String::new();
    }
    let bar = render_progress_bar(BAR_WIDTH, view.targets_done, view.targets_total);
    let queue_text = match view.queued_count {
        0 => String::new(),
        n => format!(" \u{2014} {n} more queued"),
    };
    format!(" {bar}{queue_text}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ProgressUpdate;

    fn base_view<'a>() -> JobStatusView<'a> {
        JobStatusView {
            visible: false,
            current_label: None,
            current_progress: None,
            queued_count: 0,
            targets_done: 0,
            targets_total: 0,
            summary: None,
        }
    }

    #[test]
    fn current_line_blank_when_idle() {
        assert_eq!(current_line(&base_view()), "");
    }

    #[test]
    fn current_line_shows_label_before_first_progress() {
        let view = JobStatusView {
            current_label: Some("Delete local"),
            ..base_view()
        };
        assert_eq!(current_line(&view), " Delete local...");
    }

    #[test]
    fn current_line_shows_progress_and_item() {
        let progress = ProgressUpdate {
            completed: 1,
            total: 3,
            current_item: "ct/foo".into(),
        };
        let view = JobStatusView {
            current_label: Some("Delete local"),
            current_progress: Some(&progress),
            ..base_view()
        };
        let line = current_line(&view);
        assert!(line.contains("Delete local"));
        assert!(line.contains("1/3"));
        assert!(line.contains("ct/foo"));
    }

    #[test]
    fn queue_line_blank_when_nothing_running_or_queued() {
        assert_eq!(queue_line(&base_view()), "");
    }

    #[test]
    fn queue_line_shows_queued_count() {
        let view = JobStatusView {
            current_label: Some("Push"),
            targets_done: 1,
            targets_total: 4,
            queued_count: 2,
            ..base_view()
        };
        let line = queue_line(&view);
        assert!(line.contains("1/4"));
        assert!(line.contains("2 more queued"));
    }
}
