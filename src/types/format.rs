use chrono::{DateTime, Local, Utc};

// --- Format Helpers ---

pub fn format_age(date: &DateTime<Utc>) -> String {
    let secs = Utc::now().signed_duration_since(*date).num_seconds().max(0);
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return plural(mins, "minute");
    }
    let hours = mins / 60;
    if hours < 24 {
        return plural(hours, "hour");
    }
    let days = hours / 24;
    if days < 7 {
        return plural(days, "day");
    }
    let weeks = days / 7;
    if weeks < 5 {
        return plural(weeks, "week");
    }
    let months = days / 30;
    if months < 12 {
        return plural(months, "month");
    }
    let years = days / 365;
    plural(years, "year")
}

pub fn format_age_short(date: &DateTime<Utc>) -> String {
    let secs = Utc::now().signed_duration_since(*date).num_seconds().max(0);
    if secs < 60 {
        return "now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days}d");
    }
    let weeks = days / 7;
    if weeks < 5 {
        return format!("{weeks}w");
    }
    let months = days / 30;
    if months < 12 {
        return format!("{months}mo");
    }
    let years = days / 365;
    format!("{years}y")
}

/// Format `date` in the user's local timezone, including the UTC offset.
/// Example output: `"2026-09-02 14:23:45 -07:00"`.
pub fn format_local_absolute(date: &DateTime<Utc>) -> String {
    date.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S %:z")
        .to_string()
}

fn plural(n: i64, unit: &str) -> String {
    if n == 1 {
        format!("{n} {unit} ago")
    } else {
        format!("{n} {unit}s ago")
    }
}
