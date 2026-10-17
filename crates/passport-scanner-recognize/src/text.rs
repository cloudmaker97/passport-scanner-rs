//! Turning whatever the OCR engine said into machine-readable-zone lines.
//!
//! Pure string work, so the awkward part - deciding what counts as a zone line
//! - is testable without an engine installed.

/// The line lengths ICAO 9303 defines.
const LENGTHS: [usize; 3] = [30, 36, 44];

/// Characters a zone may contain.
fn allowed(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || c == '<'
}

/// Repair the substitutions an OCR engine makes on this alphabet.
///
/// The zone has no lowercase and no punctuation, so a lowercase letter is
/// always a misread of its capital, and the usual glyph confusions can be
/// resolved in one direction only.
fn repair(c: char) -> Option<char> {
    let c = c.to_ascii_uppercase();
    Some(match c {
        '«' | '»' | '‹' | '›' | '^' | '~' => '<',
        _ if allowed(c) => c,
        ' ' => return None,
        _ => return None,
    })
}

/// Restore filler an engine dropped from the end of a line.
///
/// A long run of identical glyphs is exactly where OCR loses count, and the
/// glyph it loses here is the filler `<`, which carries no data by definition.
/// So a line that already ends in filler and falls a little short of a format
/// is padded back to it.
///
/// Names have no checksum. This restores only a filler tail; it cannot recover
/// missing personal-data letters or certify a name's spelling.
fn fit_to_length(line: &str, target: usize) -> Option<String> {
    if line.len() == target {
        return Some(line.to_owned());
    }
    let shortfall = target.checked_sub(line.len())?;
    // Only a modest amount, and only where the tail is filler already.
    if shortfall > target / 5 || !line.ends_with('<') {
        return None;
    }
    Some(format!("{line}{}", "<".repeat(shortfall)))
}

/// Bring a line to the format length it belongs to.
///
/// The target is the smallest format at least as long as the line, so a
/// thirty-character identity-card line is never stretched into a
/// thirty-six-character travel-document one. Padding only ever closes a small
/// gap to the *next* format up.
fn normalise(line: &str) -> Option<String> {
    let target = LENGTHS.iter().copied().find(|&l| l >= line.len())?;
    fit_to_length(line, target)
}

/// Whether OCR text plainly shows a zone, complete or not: at least two rows
/// that are mostly zone alphabet and contain filler runs. Printed card text
/// rarely has `<<`, so this is what justifies spending another OCR pass.
#[must_use]
pub fn looks_like_zone(ocr: &str) -> bool {
    ocr.lines()
        .take(256)
        .filter(|line| {
            let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            let zone_chars = compact.chars().filter(|&c| allowed(c)).count();
            compact.len() >= 15 && zone_chars * 10 >= compact.len() * 9 && compact.contains("<<")
        })
        .count()
        >= 2
}

/// Extract the zone from an OCR result.
///
/// Lines are kept when they fit one of the formats, and the set is only
/// accepted when it is a complete zone: three lines of thirty, or two of
/// thirty-six or forty-four.
#[must_use]
pub fn zone_lines(ocr: &str) -> Option<Vec<String>> {
    let cleaned: Vec<String> = ocr
        .lines()
        .take(256)
        .filter_map(|line| {
            // Ignore layout whitespace, but never delete an unknown glyph from the
            // middle of a field: that would shift all later fixed-width positions.
            let line: Option<String> =
                line.chars().filter(|c| !c.is_whitespace()).map(repair).collect();
            let line = line?;
            // TD1 rows lose up to a third of their filler; the repair below
            // still needs them, and only accepts what verifies.
            (18..=52).contains(&line.len()).then_some(line)
        })
        .collect();

    let mut best: Option<(bool, usize, Vec<String>)> = None;
    let mut ambiguous = false;
    for (length, count) in [(30, 3), (36, 2), (44, 2)] {
        // Only neighbouring candidate rows may form a zone. Try role orderings
        // locally because OCR can emit the three TD1 rows in the wrong order.
        for window in cleaned.windows(count) {
            let orders: &[&[usize]] = if count == 3 {
                &[&[0, 1, 2], &[0, 2, 1], &[1, 0, 2], &[1, 2, 0], &[2, 0, 1], &[2, 1, 0]]
            } else {
                &[&[0, 1], &[1, 0]]
            };
            for order in orders {
                let raw: Vec<String> = order.iter().map(|&i| window[i].clone()).collect();
                let lines = if count == 3 {
                    recover_td1_padding(&raw)
                        .unwrap_or_else(|| raw.iter().filter_map(|l| normalise(l)).collect())
                } else {
                    raw.iter().filter_map(|l| normalise(l)).collect()
                };
                if lines.len() != count || lines.iter().any(|l| l.len() != length) {
                    continue;
                }
                let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
                let Ok(doc) = passport_scanner_mrz::parse(&refs) else {
                    continue;
                };
                let rank = (
                    doc.checksums_valid(),
                    doc.fields.values().filter(|f| f.checksum_ok == Some(true)).count(),
                );
                match &best {
                    Some((valid, checks, previous)) if rank == (*valid, *checks) => {
                        if previous != &lines {
                            ambiguous = true;
                        }
                    }
                    Some((valid, checks, _)) if rank < (*valid, *checks) => {}
                    _ => {
                        best = Some((rank.0, rank.1, lines));
                        ambiguous = false;
                    }
                }
            }
        }
    }
    if ambiguous { None } else { best.map(|(_, _, lines)| lines) }
}

/// Glyphs Tesseract reports in place of a run of `<` fillers on ID cards.
/// Observed on real captures: K, E, C, S, G, Q, O, 0, 6, 8, X.
fn misread_filler(c: u8) -> bool {
    matches!(c, b'<' | b'K' | b'X' | b'E' | b'C' | b'S' | b'G' | b'Q' | b'O' | b'0' | b'6' | b'8')
}

/// A digit position read as a look-alike letter. Every such position is
/// covered by a check digit, which is what licenses the substitution.
fn as_digit(c: u8) -> Option<u8> {
    Some(match c {
        b'0'..=b'9' => c,
        b'O' | b'Q' | b'D' => b'0',
        b'I' | b'L' => b'1',
        b'Z' => b'2',
        b'S' => b'5',
        b'G' => b'6',
        b'B' => b'8',
        _ => return None,
    })
}

/// An optional-data field: real data up to the first filler pair is kept
/// verbatim; after it, only fillers and their usual misreads may follow, and
/// they all become fillers. The composite check digit covers the result.
fn optional_field(observed: &str, width: usize) -> Option<String> {
    let data_end = observed.find("<<").unwrap_or(observed.len());
    let (data, tail) = observed.split_at(data_end);
    if data.len() > width || !tail.bytes().all(misread_filler) {
        return None;
    }
    Some(format!("{data}{}", "<".repeat(width - data.len())))
}

/// A TD1 grid with filler misreads inside optional-data filler runs turned
/// back into fillers. For comparing two independently verified readings:
/// where one source saw `<` and the other a look-alike, both describe the same
/// zone. Other formats, and fields that do not fit, are returned unchanged.
#[must_use]
pub fn canonical_fillers(lines: &[String]) -> Vec<String> {
    if lines.len() != 3 || lines.iter().any(|l| l.len() != 30) {
        return lines.to_vec();
    }
    let mut out = lines.to_vec();
    if let Some(optional) = optional_field(&lines[0][15..], 15) {
        out[0] = format!("{}{optional}", &lines[0][..15]);
    }
    if let Some(optional) = optional_field(&lines[1][18..29], 11) {
        out[1] = format!("{}{optional}{}", &lines[1][..18], &lines[1][29..]);
    }
    out
}

/// German identity cards (TD1, document code `I…`, issuer `D`) leave both
/// optional-data fields empty by specification.
#[must_use]
pub fn optional_data_always_empty(lines: &[String]) -> bool {
    lines.len() == 3
        && lines.iter().all(|l| l.len() == 30)
        && lines[0].starts_with('I')
        && &lines[0][2..5] == "D<<"
}

/// Recover a TD1 zone whose rows came back with miscounted or misread
/// fillers, look-alike letters in digit positions, or noise after the name.
///
/// Every check-digited field is kept as read except for digit look-alikes
/// at digit positions; personal letters, the document number and the name
/// are never edited. The result is accepted only if every individual and the
/// composite check digit verify. A grid that already verifies as read is
/// returned untouched, so real optional data that happens to collide with a
/// filler under the checksum is never rewritten.
fn recover_td1_padding(raw: &[String]) -> Option<Vec<String>> {
    if raw.len() != 3 || raw.iter().any(|l| !(18..=40).contains(&l.len())) {
        return None;
    }
    let refs: Vec<&str> = raw.iter().map(String::as_str).collect();
    if passport_scanner_mrz::parse(&refs).is_ok_and(|d| d.checksums_valid()) {
        return Some(raw.to_vec());
    }
    let (first, second, third) = (raw[0].as_bytes(), raw[1].as_bytes(), raw[2].as_bytes());

    // Row 1: document code, state, number, number check digit, optional data.
    if first.len() < 15 {
        return None;
    }
    let mut row1 = raw[0][..14].to_owned();
    row1.push(as_digit(first[14])? as char);
    row1.push_str(&optional_field(&raw[0][15..], 15)?);

    // Row 2: birth date and check, sex, expiry and check, nationality,
    // optional data, composite check digit (always the last glyph read).
    if second.len() < 19 {
        return None;
    }
    let mut row2 = String::with_capacity(30);
    for (i, &c) in second[..15].iter().enumerate() {
        row2.push(if i == 7 { c } else { as_digit(c)? } as char);
    }
    row2.push_str(&raw[1][15..18]);
    row2.push_str(&optional_field(&raw[1][18..second.len() - 1], 11)?);
    row2.push(as_digit(second[second.len() - 1])? as char);

    // Row 3: the name, verbatim up to the filler run after the given names.
    let secondary_start = raw[2].find("<<")? + 2;
    let padding_start = secondary_start + raw[2][secondary_start..].find("<<")?;
    if padding_start > 28 || !third[padding_start..].iter().copied().all(misread_filler) {
        return None;
    }
    let row3 = format!("{}{}", &raw[2][..padding_start], "<".repeat(30 - padding_start));

    let repaired = vec![row1, row2, row3];
    let refs: Vec<&str> = repaired.iter().map(String::as_str).collect();
    passport_scanner_mrz::parse(&refs).ok()?.checksums_valid().then_some(repaired)
}

/// How much of `line` is plausible zone text, as a fraction.
///
/// Useful for deciding whether a strip is worth running through a parser at
/// all, without committing to it being correct.
///
/// Case is *not* normalised here, deliberately. A zone contains no lowercase,
/// so a line full of it is ordinary print rather than a zone - folding case
/// first would score a sentence as a near-perfect zone.
#[must_use]
pub fn zone_likeness(line: &str) -> f32 {
    if line.is_empty() {
        return 0.0;
    }
    let good = line.chars().filter(|c| allowed(*c)).count();
    good as f32 / line.chars().count() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPECIMEN: &str = "\
I<UTOD231458907<<<<<<<<<<<<<<<
7408122F1204159UTO<<<<<<<<<<<6
ERIKSSON<<ANNA<MARIA<<<<<<<<<<";

    #[test]
    fn counted_and_misread_padding_is_recovered_without_editing_personal_data() {
        let rows: Vec<&str> = SPECIMEN.lines().collect();
        let noisy = format!(
            "{}<<<<<<<<<<K<<\n{}<<<<<<<<<K<<6\nERIKSSON<<ANNA<MARIA<<<<<<<K<",
            &rows[0][..15],
            &rows[1][..18]
        );
        let lines = zone_lines(&noisy).unwrap();
        assert_eq!(&lines[0][..15], &rows[0][..15]);
        assert_eq!(&lines[1][..18], &rows[1][..18]);
        assert_eq!(lines[2], rows[2]);
        // Inside a filler run a lone K is Tesseract's chevron; the grid as
        // read did not verify, and the composite check covers the repair.
        assert_eq!(lines[0], rows[0]);
        assert_eq!(lines[1], rows[1]);
    }

    /// Shapes seen on a German identity card back, rebuilt on the ICAO
    /// specimen so no real personal data is embedded here.
    #[test]
    fn real_capture_misreads_are_repaired_only_where_check_digits_cover_them() {
        let rows: Vec<&str> = SPECIMEN.lines().collect();
        // Short first row, composite check digit read as Z-like look-alike
        // after a misread filler tail, name followed by chevron noise.
        let ocr = format!(
            "{}<<<<<<<KKK\n{}<<<<<<<KC6\n{}",
            &rows[0][..15],
            &rows[1][..18],
            "ERIKSSON<<ANNA<MARIA<<<<<<<<6OG6"
        );
        assert_eq!(zone_lines(&ocr).unwrap().join("\n"), SPECIMEN);
        // A digit look-alike in a date is repaired because the date's own
        // check digit verifies it.
        let lookalike =
            SPECIMEN.replacen("7408122", "74O8122", 1).replacen("<<<<<<<<<<<6", "<<<<<<<<<K<6", 1);
        assert_eq!(zone_lines(&lookalike).unwrap().join("\n"), SPECIMEN);
        // The same look-alike in the document number is not guessed.
        assert!(zone_lines(&SPECIMEN.replacen("D231458907<", "D23I458907<K", 1)).is_none());
        // Real optional data ahead of the filler run survives the repair.
        let mut with_data: Vec<String> = SPECIMEN.lines().map(str::to_owned).collect();
        with_data[0].replace_range(15..18, "AB1");
        let composite = format!(
            "{}{}{}{}",
            &with_data[0][5..],
            &with_data[1][..7],
            &with_data[1][8..15],
            &with_data[1][18..29]
        );
        let cd = passport_scanner_mrz::check::check_digit(&composite).unwrap().to_string();
        with_data[1].replace_range(29..30, &cd);
        let noisy = format!("{}<<<<<<<KE\n{}\n{}", &with_data[0][..18], with_data[1], with_data[2]);
        assert_eq!(zone_lines(&noisy).unwrap(), with_data);
    }

    #[test]
    fn zone_shaped_text_is_told_apart_from_card_print() {
        assert!(looks_like_zone("I<UTOD231458907<<<<<<<<\n7408122F1204159UTO<<<<<<<<\nX"));
        assert!(!looks_like_zone("BUNDESREPUBLIK DEUTSCHLAND\nPERSONALAUSWEIS\n<<"));
        assert!(!looks_like_zone("I<UTOD231458907<<<<<<<<<<<<<<<"));
        assert!(!looks_like_zone(""));
    }

    #[test]
    fn real_optional_data_and_single_letter_names_survive_recovery() {
        let mut rows: Vec<String> = SPECIMEN.lines().map(str::to_owned).collect();
        rows[0].replace_range(20..21, "K");
        let composite =
            format!("{}{}{}{}", &rows[0][5..], &rows[1][..7], &rows[1][8..15], &rows[1][18..29]);
        let cd = passport_scanner_mrz::check::check_digit(&composite).unwrap().to_string();
        rows[1].replace_range(29..30, &cd);
        let original = rows.join("\n");
        assert_eq!(zone_lines(&original).unwrap().join("\n"), original);
    }

    #[test]
    fn filler_recovery_requires_checks_and_does_not_guess_name_letters() {
        let rows: Vec<&str> = SPECIMEN.lines().collect();
        let mut bad: Vec<String> = rows.iter().map(|l| l.to_string()).collect();
        bad[0].replace_range(5..6, "E");
        assert!(recover_td1_padding(&bad).is_none());
        bad = rows.iter().map(|l| l.to_string()).collect();
        bad[2].replace_range(27..28, "H");
        assert!(zone_lines(&bad.join("\n")).is_none());
    }

    #[test]
    fn permuted_td1_rows_are_assigned_by_structure() {
        let rows: Vec<&str> = SPECIMEN.lines().collect();
        for order in [[0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
            let ocr = order.map(|i| rows[i]).join("\n");
            assert_eq!(zone_lines(&ocr).unwrap(), rows);
        }
    }

    #[test]
    fn document_numbers_cannot_become_names() {
        let rows: Vec<&str> = SPECIMEN.lines().collect();
        let bad = format!("{}\n{}\n{}", rows[0], rows[1], rows[0]);
        assert!(zone_lines(&bad).is_none());
        assert!(zone_lines(&format!("{}\n{}\nNIS<<<", rows[0], rows[1])).is_none());
    }

    #[test]
    fn same_length_chatter_is_not_a_td1_row() {
        let noisy = format!("{}\n{SPECIMEN}", "9".repeat(30));
        assert_eq!(zone_lines(&noisy).unwrap().join("\n"), SPECIMEN);
    }

    #[test]
    fn unknown_glyphs_are_not_deleted_to_shift_columns() {
        assert!(zone_lines(&SPECIMEN.replacen("D231", "D?231", 1)).is_none());
    }

    #[test]
    fn single_letter_name_components_are_never_replaced_by_fillers() {
        let rows: Vec<&str> = SPECIMEN.lines().collect();
        let name = format!("{:<30}", "EXAMPLE<<K<X").replace(' ', "<");
        let ocr = format!("{}\n{}\n{name}", rows[0], rows[1]);
        let lines = zone_lines(&ocr).unwrap();
        assert_eq!(lines[2], name);
    }

    #[test]
    fn two_equally_supported_name_rows_are_ambiguous() {
        let other = SPECIMEN.replace("ANNA", "ANNE");
        assert!(zone_lines(&format!("{SPECIMEN}\n{other}")).is_none());
    }

    #[test]
    fn all_missing_row_cases_are_rejected() {
        let rows: Vec<&str> = SPECIMEN.lines().collect();
        for missing in 0..3 {
            let ocr = rows
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != missing)
                .map(|(_, l)| *l)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(zone_lines(&ocr).is_none());
        }
    }

    #[test]
    fn valid_checks_beat_an_earlier_corrupt_candidate() {
        let bad = SPECIMEN.replace("D231458907", "D231458908");
        assert_eq!(zone_lines(&format!("{bad}\n{SPECIMEN}")).unwrap().join("\n"), SPECIMEN);
    }

    #[test]
    fn a_clean_specimen_comes_through_whole() {
        let lines = zone_lines(SPECIMEN).expect("three lines of thirty");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "I<UTOD231458907<<<<<<<<<<<<<<<");
        assert_eq!(lines[2], "ERIKSSON<<ANNA<MARIA<<<<<<<<<<");
    }

    #[test]
    fn surrounding_chatter_is_discarded() {
        let noisy = format!("Bundesrepublik Deutschland\n\n{SPECIMEN}\n\nsome footer");
        let lines = zone_lines(&noisy).expect("the zone is still found");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn chevrons_misread_as_guillemets_are_repaired() {
        let bent = SPECIMEN.replace('<', "«");
        let lines = zone_lines(&bent).expect("repaired and found");
        assert_eq!(lines[0], "I<UTOD231458907<<<<<<<<<<<<<<<");
    }

    #[test]
    fn lowercase_is_lifted_because_a_zone_has_none() {
        let lower = SPECIMEN.to_lowercase();
        let lines = zone_lines(&lower).expect("found");
        assert_eq!(lines[2], "ERIKSSON<<ANNA<MARIA<<<<<<<<<<");
    }

    #[test]
    fn a_passport_zone_is_recognised_by_its_own_shape() {
        let td3 = "\
P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<
L898902C36UTO7408122F1204159ZE184226B<<<<<10";
        let lines = zone_lines(td3).expect("two lines of forty-four");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), 44);
    }

    #[test]
    fn an_incomplete_zone_is_refused_rather_than_half_parsed() {
        let one_line = "I<UTOD231458907<<<<<<<<<<<<<<<";
        assert!(zone_lines(one_line).is_none());
    }

    #[test]
    fn dropped_trailing_filler_is_restored() {
        // Exactly what Tesseract does to a long run of chevrons: it loses
        // count at the end of the line.
        let clipped = "\
I<UTOD231458907<<<<<<<<<<
7408122F1204159UTO<<<<<<<<<<<6
ERIKSSON<<ANNA<MARIA<<<<<<<<<<";
        let lines = zone_lines(clipped).expect("the short line is padded back");
        assert_eq!(lines[0], "I<UTOD231458907<<<<<<<<<<<<<<<");
        assert!(lines.iter().all(|l| l.len() == 30));
    }

    #[test]
    fn a_line_short_of_data_is_not_padded() {
        // The shortfall is not filler, so nothing may be invented here.
        assert_eq!(fit_to_length("ERIKSSON<<ANNA<MARIA", 30), None);
    }

    #[test]
    fn a_wildly_short_line_is_not_rescued() {
        assert_eq!(fit_to_length("I<UTO<", 30), None);
    }

    #[test]
    fn padding_leaves_a_correct_line_alone() {
        let whole = "ERIKSSON<<ANNA<MARIA<<<<<<<<<<";
        assert_eq!(fit_to_length(whole, 30).as_deref(), Some(whole));
    }

    #[test]
    fn an_identity_card_line_is_never_stretched_into_a_travel_document_one() {
        // Thirty characters of filler-ended text would fit the padding rule for
        // thirty-six; it must stay at thirty.
        let card = "ERIKSSON<<ANNA<MARIA<<<<<<<<<<";
        assert_eq!(normalise(card).as_deref(), Some(card));
        assert_eq!(normalise(card).map(|l| l.len()), Some(30));
    }

    #[test]
    fn a_line_longer_than_every_format_is_discarded() {
        assert_eq!(normalise(&"A".repeat(50)), None);
    }

    #[test]
    fn likeness_separates_a_zone_from_ordinary_text() {
        assert!(zone_likeness("ERIKSSON<<ANNA<MARIA<<<<<<<<<<") > 0.99);
        // Ordinary print scores low because the zone has no lowercase at all.
        assert!(zone_likeness("Bundesrepublik Deutschland") < 0.2);
        assert_eq!(zone_likeness(""), 0.0);
    }
}
