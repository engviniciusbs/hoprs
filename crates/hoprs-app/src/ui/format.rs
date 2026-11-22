//! How file data reads in the panes: sizes, dates and permissions. Pure functions; the
//! caller passes the current year, so nothing here reads the clock.

use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, NaiveDateTime, Timelike};
use hoprs_core::EntryKind;

use crate::i18n::{self, Language};
use crate::ui::strings;

/// `68 B`, `4.1 KB`, `812 KB`, `38 MB`: base 1024, one decimal below 10.
pub fn size(bytes: u64) -> String {
    size_in(i18n::language(), bytes)
}

/// [`size`] in a given language: the decimal separator follows it (`4,1 KB` in pt-BR).
pub fn size_in(lang: Language, bytes: u64) -> String {
    let unit_name = |unit| strings::size_unit_in(lang, unit);
    const UNITS: usize = 7;
    if bytes < 1024 {
        return format!("{bytes} {}", unit_name(0));
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS {
        value /= 1024.0;
        unit += 1;
    }
    // What the reader sees, rounded; 1023.6 KB reads as 1.0 MB, not 1024 KB.
    let mut shown = if value < 10.0 {
        (value * 10.0).round() / 10.0
    } else {
        value.round()
    };
    if shown >= 1024.0 && unit + 1 < UNITS {
        value /= 1024.0;
        unit += 1;
        shown = (value * 10.0).round() / 10.0;
    }
    if shown < 10.0 {
        let number = format!("{shown:.1}");
        let number = match lang {
            Language::En => number,
            Language::PtBr | Language::Tr => number.replace('.', ","),
        };
        format!("{number} {}", unit_name(unit))
    } else {
        format!("{shown:.0} {}", unit_name(unit))
    }
}

/// `11 Nov 14:02` in `this_year`, `02 Oct 2024` otherwise.
pub fn date(at: NaiveDateTime, this_year: i32) -> String {
    date_in(i18n::language(), at, this_year)
}

/// [`date`] in a given language (`11 nov 14:02` in pt-BR).
pub fn date_in(lang: Language, at: NaiveDateTime, this_year: i32) -> String {
    let month = strings::month_in(lang, at.month0());
    if at.year() == this_year {
        format!(
            "{:02} {month} {:02}:{:02}",
            at.day(),
            at.hour(),
            at.minute()
        )
    } else {
        format!("{:02} {month} {}", at.day(), at.year())
    }
}

/// The modification time in local time, as [`date`]; empty when the server gave none.
pub fn modified(time: Option<SystemTime>, this_year: i32) -> String {
    time.map(|time| date(DateTime::<Local>::from(time).naive_local(), this_year))
        .unwrap_or_default()
}

/// The current year in local time, for [`modified`].
pub fn current_year() -> i32 {
    Local::now().year()
}

/// `drwxr-xr-x` from the entry's kind and its POSIX mode bits (setuid, setgid and
/// sticky included); empty when the server gave no mode.
pub fn permissions(kind: EntryKind, mode: Option<u32>) -> String {
    let Some(mode) = mode else {
        return String::new();
    };
    let kind = match kind {
        EntryKind::Dir => 'd',
        EntryKind::Symlink => 'l',
        EntryKind::File => '-',
    };
    let bit = |mask: u32, c: char| if mode & mask != 0 { c } else { '-' };
    // The execute slot also shows setuid / setgid / sticky.
    let exec = |x: u32, special: u32, set: char| match (mode & x != 0, mode & special != 0) {
        (true, true) => set,
        (false, true) => set.to_ascii_uppercase(),
        (true, false) => 'x',
        (false, false) => '-',
    };
    [
        kind,
        bit(0o400, 'r'),
        bit(0o200, 'w'),
        exec(0o100, 0o4000, 's'),
        bit(0o040, 'r'),
        bit(0o020, 'w'),
        exec(0o010, 0o2000, 's'),
        bit(0o004, 'r'),
        bit(0o002, 'w'),
        exec(0o001, 0o1000, 't'),
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    #[test]
    fn size_and_date_format() {
        // Sizes: bytes as they are, then base 1024 with one decimal below 10.
        assert_eq!(size(0), "0 B");
        assert_eq!(size(68), "68 B");
        assert_eq!(size(1023), "1023 B");
        assert_eq!(size(1024), "1.0 KB");
        assert_eq!(size(4198), "4.1 KB"); // 4.0996
        assert_eq!(size(4096), "4.0 KB");
        assert_eq!(size(10 * 1024 - 1), "10 KB"); // 9.999 rounds out of the decimal range
        assert_eq!(size(812 * 1024), "812 KB");
        assert_eq!(size(96 * 1024 + 300), "96 KB");
        assert_eq!(size(1024 * 1024 - 1), "1.0 MB"); // 1023.999 KB rounds up a unit
        assert_eq!(size(38 * 1024 * 1024 + 123_456), "38 MB");
        assert_eq!(size(5 * 1024u64.pow(3) / 2), "2.5 GB");
        assert_eq!(size(u64::MAX), "16 EB");

        // Dates: the year only when it is not the current one; zero-padded day.
        assert_eq!(date(at(2026, 11, 11, 14, 2), 2026), "11 Nov 14:02");
        assert_eq!(date(at(2026, 10, 2, 9, 11), 2026), "02 Oct 09:11");
        assert_eq!(date(at(2024, 10, 2, 9, 11), 2026), "02 Oct 2024");
        assert_eq!(date(at(2027, 1, 1, 0, 0), 2026), "01 Jan 2027");
        assert_eq!(date(at(2026, 12, 31, 23, 59), 2026), "31 Dec 23:59");

        // No time from the server: nothing shown.
        assert_eq!(modified(None, 2026), "");
        // A real time goes through local time to the same format.
        let shown = modified(Some(SystemTime::now()), current_year());
        assert_eq!(shown.len(), "11 Nov 14:02".len(), "{shown}");
    }

    #[test]
    fn permissions_format() {
        assert_eq!(permissions(EntryKind::Dir, Some(0o755)), "drwxr-xr-x");
        assert_eq!(permissions(EntryKind::Dir, Some(0o770)), "drwxrwx---");
        assert_eq!(permissions(EntryKind::File, Some(0o600)), "-rw-------");
        assert_eq!(permissions(EntryKind::File, Some(0o644)), "-rw-r--r--");
        assert_eq!(permissions(EntryKind::File, Some(0o640)), "-rw-r-----");
        assert_eq!(permissions(EntryKind::Symlink, Some(0o777)), "lrwxrwxrwx");
        // The file-type bits SFTP sends along are ignored; the kind decides.
        assert_eq!(permissions(EntryKind::Dir, Some(0o040_755)), "drwxr-xr-x");
        // setuid, setgid, sticky: lower case over x, upper case without it.
        assert_eq!(permissions(EntryKind::File, Some(0o4755)), "-rwsr-xr-x");
        assert_eq!(permissions(EntryKind::File, Some(0o2745)), "-rwxr-Sr-x");
        assert_eq!(permissions(EntryKind::Dir, Some(0o1777)), "drwxrwxrwt");
        assert_eq!(permissions(EntryKind::Dir, Some(0o1776)), "drwxrwxrwT");
        assert_eq!(permissions(EntryKind::File, Some(0)), "----------");
        // FTP servers may give no mode at all.
        assert_eq!(permissions(EntryKind::File, None), "");
        assert_eq!(strings::month_in(Language::En, 0), "Jan");
        assert_eq!(strings::month_in(Language::En, 11), "Dec");
    }

    #[test]
    fn pt_br_sizes_and_dates() {
        use Language::{En, PtBr};
        assert_eq!(size_in(PtBr, 4198), "4,1 KB");
        assert_eq!(size_in(PtBr, 812 * 1024), "812 KB");
        assert_eq!(size_in(PtBr, 68), "68 B");
        assert_eq!(size_in(PtBr, 5 * 1024u64.pow(3) / 2), "2,5 GB");
        assert_eq!(size_in(En, 4198), "4.1 KB");
        assert_eq!(date_in(PtBr, at(2026, 11, 11, 14, 2), 2026), "11 nov 14:02");
        assert_eq!(date_in(PtBr, at(2024, 10, 2, 9, 11), 2026), "02 out 2024");
        assert_eq!(date_in(En, at(2026, 11, 11, 14, 2), 2026), "11 Nov 14:02");
        // The active language is untouched by the explicit ones.
        assert_eq!(size(4198), "4.1 KB");
    }

    #[test]
    fn active_language_drives_the_public_functions() {
        i18n::set_language(Language::PtBr);
        assert_eq!(size(4198), "4,1 KB");
        assert_eq!(date(at(2026, 2, 3, 4, 5), 2026), "03 fev 04:05");
        assert_eq!(strings::speed(4198), "4,1 KB/s");
        i18n::set_language(Language::En);
    }
}
