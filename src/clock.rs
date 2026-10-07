use chrono::{DateTime, Datelike, FixedOffset, TimeZone, Timelike, Utc, Weekday};

pub fn beijing_offset() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("UTC+8")
}

pub fn beijing_now() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&beijing_offset())
}

pub fn weekday_zh(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "周一",
        Weekday::Tue => "周二",
        Weekday::Wed => "周三",
        Weekday::Thu => "周四",
        Weekday::Fri => "周五",
        Weekday::Sat => "周六",
        Weekday::Sun => "周日",
    }
}

pub enum Gaokao {
    Countdown {
        target: DateTime<FixedOffset>,
        days: i64,
        hours: i64,
        minutes: i64,
        seconds: i64,
        progress: f32,
    },
    InProgress {
        year: i32,
    },
}

/// 高考开考日为每年 6 月 7 日 09:00（北京时间），8 日 18:00 前视为进行中。
pub fn gaokao(now: DateTime<FixedOffset>) -> Gaokao {
    let tz = beijing_offset();
    let year = now.year();
    let start = tz.with_ymd_and_hms(year, 6, 7, 9, 0, 0).unwrap();
    let end = tz.with_ymd_and_hms(year, 6, 8, 18, 0, 0).unwrap();
    if now >= start && now < end {
        return Gaokao::InProgress { year };
    }
    let target = if now < start {
        start
    } else {
        tz.with_ymd_and_hms(year + 1, 6, 7, 9, 0, 0).unwrap()
    };
    let left = target.signed_duration_since(now);
    let year_secs = 365.0 * 24.0 * 3600.0;
    let progress = (1.0 - left.num_seconds() as f32 / year_secs).clamp(0.0, 1.0);
    Gaokao::Countdown {
        target,
        days: left.num_days(),
        hours: left.num_hours().rem_euclid(24),
        minutes: left.num_minutes().rem_euclid(60),
        seconds: left.num_seconds().rem_euclid(60),
        progress,
    }
}

/// 腾讯行情时间 `yyyyMMddHHmmss`。日期不是今天则视为休市。
pub fn market_status(quote_time: &str, now: DateTime<FixedOffset>) -> &'static str {
    if quote_time.len() < 8 {
        return "—";
    }
    let today = format!("{:04}{:02}{:02}", now.year(), now.month(), now.day());
    if &quote_time[..8] != today {
        return "休市";
    }
    let hhmm = now.hour() * 100 + now.minute();
    if (930..1130).contains(&hhmm) || (1300..1500).contains(&hhmm) {
        "交易中"
    } else if hhmm < 930 {
        "未开盘"
    } else if hhmm < 1300 {
        "午间休市"
    } else {
        "已收盘"
    }
}

pub fn format_quote_time(raw: &str) -> String {
    if raw.len() < 12 {
        return raw.to_string();
    }
    format!(
        "{}-{} {}:{}",
        &raw[4..6],
        &raw[6..8],
        &raw[8..10],
        &raw[10..12]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<FixedOffset> {
        beijing_offset().with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn countdown_to_next_june_when_past_this_year() {
        match gaokao(at(2026, 10, 6, 0, 0)) {
            Gaokao::Countdown { target, days, .. } => {
                assert_eq!(target, at(2027, 6, 7, 9, 0));
                assert_eq!(days, 244);
            }
            Gaokao::InProgress { .. } => panic!("should be counting down"),
        }
    }

    #[test]
    fn countdown_uses_this_year_before_exam() {
        match gaokao(at(2026, 1, 1, 0, 0)) {
            Gaokao::Countdown { target, .. } => assert_eq!(target, at(2026, 6, 7, 9, 0)),
            Gaokao::InProgress { .. } => panic!("should be counting down"),
        }
    }

    #[test]
    fn in_progress_during_exam_window() {
        match gaokao(at(2026, 6, 7, 10, 0)) {
            Gaokao::InProgress { year } => assert_eq!(year, 2026),
            Gaokao::Countdown { .. } => panic!("exam should be in progress"),
        }
    }

    #[test]
    fn holiday_quote_is_closed() {
        assert_eq!(market_status("20260930161500", at(2026, 10, 6, 10, 0)), "休市");
    }

    #[test]
    fn same_day_session() {
        assert_eq!(market_status("20261006101500", at(2026, 10, 6, 10, 20)), "交易中");
        assert_eq!(market_status("20261006120000", at(2026, 10, 6, 12, 0)), "午间休市");
        assert_eq!(market_status("20261006160000", at(2026, 10, 6, 16, 0)), "已收盘");
    }
}
