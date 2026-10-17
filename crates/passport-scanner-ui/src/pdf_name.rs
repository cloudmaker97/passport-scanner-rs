//! Export file names: `YYYY-MM-DD--NNN--HH-MM-SS.pdf`.
//!
//! The number counts the exports of one day in one place, starting at 001. It
//! is derived from what is already there rather than stored, so deleting or
//! moving files never leaves a stale counter behind.

/// Local wall-clock time of an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Stamp {
    /// `YYYY-MM-DD`, the part that groups a day's exports.
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    pub fn file_name(&self, number: u32) -> String {
        format!(
            "{}--{number:03}--{:02}-{:02}-{:02}.pdf",
            self.date(),
            self.hour,
            self.minute,
            self.second
        )
    }
}

/// The number of an export file for `date`, or `None` if the name is another file.
pub fn number_of(name: &str, date: &str) -> Option<u32> {
    let rest = name.strip_prefix(date)?.strip_prefix("--")?;
    let (number, time) = rest.split_once("--")?;
    let time = time.strip_suffix(".pdf").or_else(|| time.strip_suffix(".PDF"))?;
    let well_formed = time.len() == 8
        && time
            .bytes()
            .enumerate()
            .all(|(i, b)| if i == 2 || i == 5 { b == b'-' } else { b.is_ascii_digit() });
    if !well_formed || number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

/// One past the highest number already used on `date`; 1 if none is.
pub fn next_number<'a>(date: &str, existing: impl IntoIterator<Item = &'a str>) -> u32 {
    existing
        .into_iter()
        .filter_map(|name| number_of(name, date))
        .max()
        .map_or(1, |highest| highest.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAMP: Stamp = Stamp { year: 2026, month: 3, day: 7, hour: 9, minute: 5, second: 4 };

    #[test]
    fn formats_with_zero_padding() {
        assert_eq!(STAMP.file_name(1), "2026-03-07--001--09-05-04.pdf");
        assert_eq!(STAMP.file_name(1234), "2026-03-07--1234--09-05-04.pdf");
    }

    #[test]
    fn continues_after_the_highest_number_of_the_same_day_only() {
        let existing = [
            "2026-03-07--001--08-00-00.pdf",
            "2026-03-07--007--08-10-00.pdf",
            "2026-03-07--003--08-20-00.PDF",
            "2026-03-06--050--08-00-00.pdf",
            "2026-03-07--999.pdf",
            "2026-03-07--x12--08-00-00.pdf",
            "2026-03-07--012--8-0-0.pdf",
            "notes.txt",
        ];
        assert_eq!(next_number(&STAMP.date(), existing), 8);
        assert_eq!(next_number(&STAMP.date(), []), 1);
        assert_eq!(next_number("2026-03-06", existing), 51);
    }

    #[test]
    fn its_own_names_are_recognised() {
        assert_eq!(number_of(&STAMP.file_name(42), &STAMP.date()), Some(42));
    }
}
