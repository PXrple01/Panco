//! Lokale Zeit ohne Zusatzbibliothek: Unix-Sekunden + UTC-Versatz (von der Plattform) → Datum.
//! Reicht für "heute/gestern", Dateidaten und Backup-Namen.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// 0 = Montag … 6 = Sonntag
    pub weekday: u32,
}

pub fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

pub fn unix(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Tage seit 1970-01-01 → (Jahr, Monat, Tag). Algorithmus von Howard Hinnant.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + if m <= 2 { 1 } else { 0 }) as i32;
    (y, m, d)
}

pub fn to_local(unix: i64, offset_secs: i32) -> LocalDate {
    let t = unix + offset_secs as i64;
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    LocalDate {
        year,
        month,
        day,
        hour: (secs / 3600) as u32,
        minute: ((secs % 3600) / 60) as u32,
        // 1970-01-01 war ein Donnerstag (= 3)
        weekday: (days + 3).rem_euclid(7) as u32,
    }
}

/// Beginn des lokalen Tages (Unix-Sekunden), `days_back` Tage zurück.
pub fn local_midnight(unix: i64, offset_secs: i32, days_back: i64) -> i64 {
    let t = unix + offset_secs as i64;
    t.div_euclid(86_400) * 86_400 - offset_secs as i64 - days_back * 86_400
}

/// "heute 14:05", "gestern 09:12", "vor 3 Tagen", "am 12.03.2025"
pub fn relative_de(then: i64, now: i64, offset_secs: i32) -> String {
    let d = to_local(then, offset_secs);
    if then >= local_midnight(now, offset_secs, 0) {
        format!("heute {:02}:{:02}", d.hour, d.minute)
    } else if then >= local_midnight(now, offset_secs, 1) {
        format!("gestern {:02}:{:02}", d.hour, d.minute)
    } else if then >= local_midnight(now, offset_secs, 7) {
        let days = (local_midnight(now, offset_secs, 0) - local_midnight(then, offset_secs, 0)) / 86_400;
        format!("vor {days} Tagen")
    } else {
        format!("am {:02}.{:02}.{}", d.day, d.month, d.year)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        // 2026-10-26 12:30 UTC = 13:30 MEZ (+1 h), Montag
        let t = 1_793_017_800;
        let d = to_local(t, 3600);
        assert_eq!((d.year, d.month, d.day, d.hour, d.minute, d.weekday), (2026, 10, 26, 13, 30, 0));
        assert_eq!(to_local(0, 0).weekday, 3);
        assert_eq!(
            to_local(951_782_400, 0),
            LocalDate { year: 2000, month: 2, day: 29, hour: 0, minute: 0, weekday: 1 }
        );
    }

    #[test]
    fn relative() {
        let now = 1_793_017_800; // Mo 13:30 lokal
        assert_eq!(relative_de(now - 3600, now, 3600), "heute 12:30");
        assert_eq!(relative_de(now - 86_400, now, 3600), "gestern 13:30");
        assert_eq!(relative_de(now - 3 * 86_400, now, 3600), "vor 3 Tagen");
        assert_eq!(relative_de(now - 40 * 86_400, now, 3600), "am 16.09.2026");
    }
}
