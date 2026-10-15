//! Small text formatting helpers.

/// "now", "5 min", "3 h", "2 d", "4 mo", "3 y".
pub fn relative_time(then: i64, now: i64) -> String {
    let d = (now - then).max(0);
    match d {
        0..60 => "now".into(),
        60..3600 => format!("{} min", d / 60),
        3600..86_400 => format!("{} h", d / 3600),
        86_400..2_592_000 => format!("{} d", d / 86_400),
        2_592_000..31_536_000 => format!("{} mo", d / 2_592_000),
        _ => format!("{} y", d / 31_536_000),
    }
}

pub fn absolute_time(then: i64) -> String {
    chrono::DateTime::from_timestamp(then, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_buckets() {
        assert_eq!(relative_time(100, 110), "now");
        assert_eq!(relative_time(0, 300), "5 min");
        assert_eq!(relative_time(0, 3 * 3600), "3 h");
        assert_eq!(relative_time(0, 2 * 86_400), "2 d");
        assert_eq!(relative_time(0, 40 * 86_400), "1 mo");
        assert_eq!(relative_time(0, 800 * 86_400), "2 y");
        assert_eq!(
            relative_time(10, 0),
            "now",
            "clock skew never goes negative"
        );
    }
}
