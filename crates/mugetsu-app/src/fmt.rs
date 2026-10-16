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

/// "512 o", "1,5 Mo", "40 Mo", "3,0 Go" (B / KB / MB / GB in English).
pub fn human_size(bytes: u64) -> String {
    size_in(bytes, crate::i18n::is_fr())
}

fn size_in(bytes: u64, fr: bool) -> String {
    let units = if fr {
        ["o", "Ko", "Mo", "Go"]
    } else {
        ["B", "KB", "MB", "GB"]
    };
    let b = bytes as f64;
    let (v, u) = match bytes {
        0..1_024 => return format!("{bytes} {}", units[0]),
        1_024..1_048_576 => (b / 1_024., units[1]),
        1_048_576..1_073_741_824 => (b / 1_048_576., units[2]),
        _ => (b / 1_073_741_824., units[3]),
    };
    let v = if v < 10. {
        format!("{v:.1}")
    } else {
        format!("{v:.0}")
    };
    let v = if fr { v.replace('.', ",") } else { v };
    format!("{v} {u}")
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

    #[test]
    fn human_sizes() {
        assert_eq!(size_in(512, false), "512 B");
        assert_eq!(size_in(1_536, false), "1.5 KB");
        assert_eq!(size_in(1_536, true), "1,5 Ko");
        assert_eq!(size_in(40 * 1_048_576, false), "40 MB");
        assert_eq!(size_in(3 * 1_073_741_824, true), "3,0 Go");
    }
}
