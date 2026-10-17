//! The ICAO 9303 check-digit algorithm.

/// Numeric value of one MRZ character.
///
/// Digits are themselves, letters are `A = 10` through `Z = 35`, and the filler `<`
/// counts as zero. Anything else is rejected.
#[must_use]
pub fn char_value(c: char) -> Option<u32> {
    match c {
        '0'..='9' => Some(c as u32 - '0' as u32),
        'A'..='Z' => Some(c as u32 - 'A' as u32 + 10),
        '<' => Some(0),
        _ => None,
    }
}

/// Compute the check digit over `field`.
///
/// Characters are weighted `7, 3, 1` repeating, summed, and reduced modulo ten.
/// Returns `None` if the field contains a character the standard does not define.
#[must_use]
pub fn check_digit(field: &str) -> Option<u8> {
    const WEIGHTS: [u32; 3] = [7, 3, 1];
    let mut sum = 0u32;
    for (i, c) in field.chars().enumerate() {
        sum += char_value(c)? * WEIGHTS[i % 3];
    }
    Some((sum % 10) as u8)
}

/// Verify `field` against the check digit character `expected`.
///
/// A filler `<` in the check position means "not used", which the standard allows for
/// optional fields; that counts as valid.
#[must_use]
pub fn verify(field: &str, expected: char) -> bool {
    if expected == '<' {
        return field.bytes().all(|c| c == b'<');
    }
    let Some(actual) = check_digit(field) else {
        return false;
    };
    expected.to_digit(10) == Some(u32::from(actual))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_values_follow_the_standard() {
        assert_eq!(char_value('0'), Some(0));
        assert_eq!(char_value('9'), Some(9));
        assert_eq!(char_value('A'), Some(10));
        assert_eq!(char_value('Z'), Some(35));
        assert_eq!(char_value('<'), Some(0));
        assert_eq!(char_value('a'), None);
        assert_eq!(char_value('-'), None);
    }

    #[test]
    fn specimen_document_number() {
        // From the ICAO 9303 specimen passport: L898902C3 has check digit 6.
        assert_eq!(check_digit("L898902C3"), Some(6));
        assert!(verify("L898902C3", '6'));
        assert!(!verify("L898902C3", '5'));
    }

    #[test]
    fn specimen_dates() {
        // Date of birth 740812 -> 2, expiry 120415 -> 9.
        assert_eq!(check_digit("740812"), Some(2));
        assert_eq!(check_digit("120415"), Some(9));
    }

    #[test]
    fn unused_check_position_is_accepted() {
        assert!(verify("<<<<<<<<<<<<<<", '<'));
    }

    #[test]
    fn invalid_characters_fail_rather_than_panic() {
        assert_eq!(check_digit("abc"), None);
        assert!(!verify("abc", '0'));
    }
}
