//! Pattern recognition algorithms for sensitive data (PII) identification in extracted text.
//!
//! Provides fast, deterministic, zero-dependency pattern scanners for emails, phone numbers,
//! Social Security Numbers (SSN), Credit Cards (with Luhn validation), Mexican RFC, and Mexican CURP.

use crate::redact::types::RedactionPattern;

/// Scans `text` for occurrences matching the given `pattern`, returning byte offset ranges `[start..end]`.
pub fn find_matches(pattern: &RedactionPattern, text: &str) -> Vec<(usize, usize)> {
    match pattern {
        RedactionPattern::Email => find_emails(text),
        RedactionPattern::Phone => find_phones(text),
        RedactionPattern::Ssn => find_ssn(text),
        RedactionPattern::CreditCard => find_credit_cards(text),
        RedactionPattern::Rfc => find_rfc(text),
        RedactionPattern::Curp => find_curp(text),
        RedactionPattern::Text {
            query,
            case_sensitive,
        } => find_substring(text, query, *case_sensitive),
    }
}

/// Finds exact substring matches with optional case sensitivity.
pub fn find_substring(text: &str, query: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
    if query.is_empty() || text.len() < query.len() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    if case_sensitive {
        for (idx, _) in text.match_indices(query) {
            matches.push((idx, idx + query.len()));
        }
    } else {
        let lower_text = text.to_lowercase();
        let lower_query = query.to_lowercase();
        for (idx, _) in lower_text.match_indices(&lower_query) {
            matches.push((idx, idx + query.len()));
        }
    }
    matches
}

/// Scans for email addresses (e.g. `john.doe@company.com`).
pub fn find_emails(text: &str) -> Vec<(usize, usize)> {
    let mut results = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();

    let is_local_char = |b: u8| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-' || b == b'+';
    let is_domain_char = |b: u8| b.is_ascii_alphanumeric() || b == b'-';

    let mut i = 0;
    while i < len {
        if bytes[i] == b'@' {
            // Scan backwards for local part
            let mut start = i;
            while start > 0 && is_local_char(bytes[start - 1]) {
                start -= 1;
            }

            // Local part must not be empty and must not start/end with dot
            if start < i && bytes[start] != b'.' && bytes[i - 1] != b'.' {
                // Scan forward for domain
                let mut end = i + 1;
                let mut has_dot = false;
                let mut last_dot = 0;

                while end < len && (is_domain_char(bytes[end]) || bytes[end] == b'.') {
                    if bytes[end] == b'.' {
                        has_dot = true;
                        last_dot = end;
                    }
                    end += 1;
                }

                // Check valid TLD after last dot (at least 2 letters, not trailing dot)
                if has_dot && last_dot > i + 1 && end > last_dot + 1 {
                    let tld = &bytes[last_dot + 1..end];
                    if tld.iter().all(|b| b.is_ascii_alphabetic()) && tld.len() >= 2 {
                        results.push((start, end));
                        i = end;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }

    results
}

/// Scans for Social Security Numbers in standard `123-45-6789` format or 9 contiguous digits.
pub fn find_ssn(text: &str) -> Vec<(usize, usize)> {
    let mut results = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();

    // Check `\b\d{3}-\d{2}-\d{4}\b` (11 chars)
    if n >= 11 {
        for i in 0..=n - 11 {
            let is_prev_digit = if i > 0 { chars[i - 1].1.is_ascii_digit() } else { false };
            let is_next_digit = if i + 11 < n { chars[i + 11].1.is_ascii_digit() } else { false };

            if !is_prev_digit && !is_next_digit {
                let matches_pattern =
                    chars[i].1.is_ascii_digit() &&
                    chars[i + 1].1.is_ascii_digit() &&
                    chars[i + 2].1.is_ascii_digit() &&
                    chars[i + 3].1 == '-' &&
                    chars[i + 4].1.is_ascii_digit() &&
                    chars[i + 5].1.is_ascii_digit() &&
                    chars[i + 6].1 == '-' &&
                    chars[i + 7].1.is_ascii_digit() &&
                    chars[i + 8].1.is_ascii_digit() &&
                    chars[i + 9].1.is_ascii_digit() &&
                    chars[i + 10].1.is_ascii_digit();

                if matches_pattern {
                    let start_byte = chars[i].0;
                    let end_byte = if i + 11 < n { chars[i + 11].0 } else { text.len() };
                    results.push((start_byte, end_byte));
                }
            }
        }
    }

    results
}

/// Scans for Credit Card PAN numbers (16 digits formatted `4-4-4-4` or continuous 16 digits) with Luhn validation.
pub fn find_credit_cards(text: &str) -> Vec<(usize, usize)> {
    let mut results = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();

    // 1. Formatted: 4 digits + sep + 4 digits + sep + 4 digits + sep + 4 digits (19 chars)
    if n >= 19 {
        for i in 0..=n - 19 {
            let sep1 = chars[i + 4].1;
            let sep2 = chars[i + 9].1;
            let sep3 = chars[i + 14].1;

            if (sep1 == '-' || sep1 == ' ') && sep1 == sep2 && sep2 == sep3 {
                let is_digits = (0..4).all(|k| chars[i + k].1.is_ascii_digit())
                    && (5..9).all(|k| chars[i + k].1.is_ascii_digit())
                    && (10..14).all(|k| chars[i + k].1.is_ascii_digit())
                    && (15..19).all(|k| chars[i + k].1.is_ascii_digit());

                if is_digits {
                    // Extract digits for Luhn check
                    let digits: Vec<u32> = (0..19)
                        .filter(|&k| k != 4 && k != 9 && k != 14)
                        .filter_map(|k| chars[i + k].1.to_digit(10))
                        .collect();

                    if digits.len() == 16 && luhn_check(&digits) {
                        let start_byte = chars[i].0;
                        let end_byte = if i + 19 < n { chars[i + 19].0 } else { text.len() };
                        results.push((start_byte, end_byte));
                    }
                }
            }
        }
    }

    // 2. Unformatted: continuous 16 digits bounded by non-digits
    if n >= 16 {
        for i in 0..=n - 16 {
            let is_prev_digit = if i > 0 { chars[i - 1].1.is_ascii_digit() } else { false };
            let is_next_digit = if i + 16 < n { chars[i + 16].1.is_ascii_digit() } else { false };

            if !is_prev_digit && !is_next_digit {
                let is_all_digits = (0..16).all(|k| chars[i + k].1.is_ascii_digit());
                if is_all_digits {
                    let digits: Vec<u32> = (0..16)
                        .filter_map(|k| chars[i + k].1.to_digit(10))
                        .collect();

                    if luhn_check(&digits) {
                        let start_byte = chars[i].0;
                        let end_byte = if i + 16 < n { chars[i + 16].0 } else { text.len() };
                        results.push((start_byte, end_byte));
                    }
                }
            }
        }
    }

    results
}

/// Verifies digits against the standard Luhn checksum algorithm (Mod 10).
fn luhn_check(digits: &[u32]) -> bool {
    let mut sum = 0;
    let mut alternate = false;
    for &d in digits.iter().rev() {
        let mut val = d;
        if alternate {
            val *= 2;
            if val > 9 {
                val -= 9;
            }
        }
        sum += val;
        alternate = !alternate;
    }
    sum % 10 == 0
}

/// Scans for standard phone numbers (domestic `(555) 123-4567`, `555-123-4567`, international `+1-555...` or `+52 55...`).
pub fn find_phones(text: &str) -> Vec<(usize, usize)> {
    let mut results = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();

    let mut i = 0;
    while i < n {
        // Phone numbers typically start with '+' or '(' or a digit
        let c = chars[i].1;
        if c == '+' || c == '(' || c.is_ascii_digit() {
            let start = i;
            let mut end = i;
            let mut digit_count = 0;

            while end < n {
                let ch = chars[end].1;
                if ch.is_ascii_digit() {
                    digit_count += 1;
                    end += 1;
                } else if ch == '-' || ch == ' ' || ch == '(' || ch == ')' || ch == '.' || ch == '+' {
                    end += 1;
                } else {
                    break;
                }
            }

            // Trim trailing punctuation from candidate
            while end > start && !chars[end - 1].1.is_ascii_digit() {
                end -= 1;
            }

            // Valid phone typically has 7 to 15 digits
            if digit_count >= 7 && digit_count <= 15 && (end - start) >= 7 {
                let start_byte = chars[start].0;
                let end_byte = if end < n { chars[end].0 } else { text.len() };
                results.push((start_byte, end_byte));
                i = end;
                continue;
            }
        }
        i += 1;
    }

    results
}

/// Scans for Mexican RFC (Tax ID): 3-4 letters + 6 digits (YYMMDD) + 3 alphanumeric homoclave (12 or 13 chars).
pub fn find_rfc(text: &str) -> Vec<(usize, usize)> {
    let mut results = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();

    let is_rfc_letter = |c: char| c.is_ascii_alphabetic() || c == '&' || c == 'Ñ' || c == 'ñ';

    for i in 0..n {
        let is_prev_alnum = if i > 0 { chars[i - 1].1.is_ascii_alphanumeric() } else { false };
        if is_prev_alnum {
            continue;
        }

        // Try Persona Física (13 chars: 4 letters + 6 digits + 3 alnum)
        if i + 13 <= n {
            let is_next_alnum = if i + 13 < n { chars[i + 13].1.is_ascii_alphanumeric() } else { false };
            if !is_next_alnum {
                let letters_4 = (0..4).all(|k| is_rfc_letter(chars[i + k].1));
                let digits_6 = (4..10).all(|k| chars[i + k].1.is_ascii_digit());
                let homoclave_3 = (10..13).all(|k| chars[i + k].1.is_ascii_alphanumeric());

                if letters_4 && digits_6 && homoclave_3 {
                    let start_byte = chars[i].0;
                    let end_byte = if i + 13 < n { chars[i + 13].0 } else { text.len() };
                    results.push((start_byte, end_byte));
                    continue;
                }
            }
        }

        // Try Persona Moral (12 chars: 3 letters + 6 digits + 3 alnum)
        if i + 12 <= n {
            let is_next_alnum = if i + 12 < n { chars[i + 12].1.is_ascii_alphanumeric() } else { false };
            if !is_next_alnum {
                let letters_3 = (0..3).all(|k| is_rfc_letter(chars[i + k].1));
                let digits_6 = (3..9).all(|k| chars[i + k].1.is_ascii_digit());
                let homoclave_3 = (9..12).all(|k| chars[i + k].1.is_ascii_alphanumeric());

                if letters_3 && digits_6 && homoclave_3 {
                    let start_byte = chars[i].0;
                    let end_byte = if i + 12 < n { chars[i + 12].0 } else { text.len() };
                    results.push((start_byte, end_byte));
                }
            }
        }
    }

    results
}

/// Scans for Mexican CURP (18 characters: 4 letters + 6 digits + 'H'/'M' + 5 letters + 1 alnum + 1 digit).
pub fn find_curp(text: &str) -> Vec<(usize, usize)> {
    let mut results = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();

    if n < 18 {
        return results;
    }

    for i in 0..=n - 18 {
        let is_prev_alnum = if i > 0 { chars[i - 1].1.is_ascii_alphanumeric() } else { false };
        let is_next_alnum = if i + 18 < n { chars[i + 18].1.is_ascii_alphanumeric() } else { false };

        if !is_prev_alnum && !is_next_alnum {
            let letters_4 = (0..4).all(|k| chars[i + k].1.is_ascii_alphabetic());
            let digits_6 = (4..10).all(|k| chars[i + k].1.is_ascii_digit());
            let gender = chars[i + 10].1.to_ascii_uppercase();
            let is_gender = gender == 'H' || gender == 'M';
            let state_and_cons = (11..16).all(|k| chars[i + k].1.is_ascii_alphabetic());
            let homoclave = chars[i + 16].1.is_ascii_alphanumeric();
            let check_digit = chars[i + 17].1.is_ascii_digit();

            if letters_4 && digits_6 && is_gender && state_and_cons && homoclave && check_digit {
                let start_byte = chars[i].0;
                let end_byte = if i + 18 < n { chars[i + 18].0 } else { text.len() };
                results.push((start_byte, end_byte));
            }
        }
    }

    results
}
