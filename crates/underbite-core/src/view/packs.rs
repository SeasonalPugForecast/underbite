//! Translations of the strings table. The only pack is the pseudo-locale, built from the English
//! table: every letter accented and every text 40 % longer, `{placeholders}` kept. It finds text
//! written outside the table and layouts that cut or glue words. Tests and debug builds only.

use std::sync::OnceLock;

use crate::view::strings::{Key, SafetyKey};

const LOWER: [char; 26] = [
    'ȧ', 'ƀ', 'ç', 'đ', 'é', 'ƒ', 'ğ', 'ħ', 'í', 'ĵ', 'ķ', 'ł', 'ɱ', 'ñ', 'ö', 'þ', 'ʠ', 'ŕ', 'š',
    'ŧ', 'ü', 'ṽ', 'ŵ', 'ẋ', 'ý', 'ž',
];
const UPPER: [char; 26] = [
    'Å', 'Ɓ', 'Ç', 'Đ', 'É', 'Ƒ', 'Ğ', 'Ħ', 'Í', 'Ĵ', 'Ķ', 'Ł', 'Ṁ', 'Ñ', 'Ö', 'Þ', 'Ǫ', 'Ŕ', 'Š',
    'Ŧ', 'Ü', 'Ṽ', 'Ŵ', 'Ẋ', 'Ý', 'Ž',
];

/// One letter in the pseudo-locale; anything but an ASCII letter stays.
fn accent(c: char) -> char {
    let index = |base: u8| usize::from(c as u8 - base);
    match c {
        'a'..='z' => LOWER[index(b'a')],
        'A'..='Z' => UPPER[index(b'A')],
        other => other,
    }
}

/// A text in the pseudo-locale: "[Ŧíđé ~~]". Letters outside `{placeholders}` are accented and
/// the text is wrapped and padded to at least 1.4 times its length without the placeholders.
pub fn pseudo(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2 + 4);
    out.push('[');
    let mut shown = 0usize;
    let mut in_placeholder = false;
    for c in text.chars() {
        match c {
            '{' => in_placeholder = true,
            '}' => in_placeholder = false,
            _ => {}
        }
        if in_placeholder || c == '}' {
            out.push(c);
        } else {
            out.push(accent(c));
            shown += 1;
        }
    }
    // The brackets and the space count towards the 40 %; the tildes make up the rest.
    let extra = (shown * 2).div_ceil(5).max(3);
    out.push(' ');
    out.extend(std::iter::repeat_n('~', extra - 3));
    out.push(']');
    out
}

/// A playful text in the pseudo-locale.
pub(super) fn key(key: Key, luna: bool) -> &'static str {
    static TABLE: OnceLock<Vec<[String; 2]>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        Key::ALL
            .iter()
            .map(|k| {
                let (plain, luna) = k.english();
                [pseudo(plain), pseudo(luna)]
            })
            .collect()
    });
    table
        .get(key as usize)
        .map_or_else(|| key.english().0, |pair| pair[usize::from(luna)].as_str())
}

/// A safety text in the pseudo-locale: one text, as in English.
pub(super) fn safety(key: SafetyKey) -> &'static str {
    static TABLE: OnceLock<Vec<String>> = OnceLock::new();
    let table = TABLE.get_or_init(|| SafetyKey::ALL.iter().map(|k| pseudo(k.text())).collect());
    table.get(key as usize).map_or(key.text(), String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `{name}` placeholders in a text, sorted.
    fn placeholders(text: &str) -> Vec<&str> {
        let mut out: Vec<&str> = text
            .split('{')
            .skip(1)
            .filter_map(|s| s.split_once('}').map(|(name, _)| name))
            .collect();
        out.sort_unstable();
        out
    }

    /// The length without placeholders.
    fn shown(text: &str) -> usize {
        let mut n = 0;
        let mut inside = false;
        for c in text.chars() {
            match c {
                '{' => inside = true,
                '}' => inside = false,
                _ if !inside => n += 1,
                _ => {}
            }
        }
        n
    }

    #[test]
    fn pseudo_accents_every_letter_and_keeps_placeholders() {
        assert_eq!(pseudo("Tide"), "[Ŧíđé ]");
        assert_eq!(pseudo("Next {kind} {time}"), "[Ñéẋŧ {kind} {time} ]");
        assert_eq!(pseudo("Water temperature"), "[Ŵȧŧéŕ ŧéɱþéŕȧŧüŕé ~~~~]");
        assert_eq!(pseudo("—"), "[— ]");
        for c in ('a'..='z').chain('A'..='Z') {
            let a = accent(c);
            assert!(!a.is_ascii() && a.is_alphabetic(), "{c}");
        }
    }

    #[test]
    fn every_text_is_translated_once_longer_and_with_its_placeholders() {
        let mut seen = std::collections::BTreeSet::new();
        let pairs = Key::ALL
            .iter()
            .flat_map(|&k| {
                let (plain, luna) = k.english();
                [(plain, key(k, false)), (luna, key(k, true))]
            })
            .chain(SafetyKey::ALL.iter().map(|&k| (k.text(), safety(k))));
        for (english, translated) in pairs {
            assert_ne!(english, translated);
            assert_eq!(placeholders(english), placeholders(translated), "{english}");
            assert!(
                shown(translated) * 5 >= shown(english) * 7,
                "{translated} is not 40 % longer than {english}"
            );
            assert!(
                !translated
                    .split(['{', '}'])
                    .step_by(2)
                    .any(|part| part.chars().any(|c| c.is_ascii_alphabetic())),
                "{translated}"
            );
            seen.insert(translated);
        }
        assert!(seen.len() > Key::ALL.len());
    }
}
