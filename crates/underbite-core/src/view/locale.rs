//! The locale the view writes in: which words it uses and how it writes numbers and dates.
//!
//! The locale is a parameter of the view input, like the units and the clock: the core never
//! reads the system's settings, so the same input always gives the same text.

use whether_core::view::format::number;

/// Which words the view uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Language {
    #[default]
    English,
    /// Every text of the table with its letters accented and made 40 % longer, so that a word
    /// written outside the table, a cut-off label or a sentence glued from pieces stands out.
    /// Only tests and debug builds carry its pack; a release build writes English instead.
    Pseudo,
}

/// How a number is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NumberStyle {
    /// "1,234.5"
    #[default]
    Point,
    /// "1.234,5"
    Comma,
    /// "1 234,5", the thousands set apart by a narrow no-break space.
    SpaceComma,
}

impl NumberStyle {
    /// The decimal mark and the thousands separator.
    const fn marks(self) -> (char, char) {
        match self {
            Self::Point => ('.', ','),
            Self::Comma => (',', '.'),
            Self::SpaceComma => (',', '\u{202F}'),
        }
    }
}

/// The order of the day and the month in a short date.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DateOrder {
    /// "Nov 8"
    #[default]
    MonthDay,
    /// "8 Nov"
    DayMonth,
}

/// The words and the number and date conventions of the view's text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Locale {
    pub language: Language,
    pub numbers: NumberStyle,
    pub dates: DateOrder,
}

impl Locale {
    /// English words, "1,234.5" and "Nov 8".
    pub const ENGLISH: Self = Self {
        language: Language::English,
        numbers: NumberStyle::Point,
        dates: DateOrder::MonthDay,
    };

    /// The pseudo-locale: accented words, "1 234,5" and "8 Nov". For tests and debug builds.
    pub const PSEUDO: Self = Self {
        language: Language::Pseudo,
        numbers: NumberStyle::SpaceComma,
        dates: DateOrder::DayMonth,
    };

    /// The decimal mark: '.' or ','.
    pub const fn decimal_mark(self) -> char {
        self.numbers.marks().0
    }

    /// A number Rust wrote with a decimal point ("+0.3") with the locale's decimal mark.
    pub fn with_decimal_mark(self, text: &str) -> String {
        match self.decimal_mark() {
            '.' => text.to_owned(),
            mark => text.replace('.', &mark.to_string()),
        }
    }

    /// `value` with `decimals` places, without thousands separators: "1020", "12.4" ("12,4" with
    /// a decimal comma). Never "-0".
    pub fn number(self, value: f32, decimals: u8) -> String {
        self.with_decimal_mark(&number(value, decimals))
    }

    /// `value` with `decimals` places and the thousands grouped: "1,500", "12,345.6".
    pub fn grouped(self, value: f32, decimals: u8) -> String {
        let text = number(value, decimals);
        let (decimal, group) = self.numbers.marks();
        match text.split_once('.') {
            Some((whole, fraction)) => {
                format!("{}{decimal}{fraction}", group_thousands(whole, group))
            }
            None => group_thousands(&text, group),
        }
    }
}

/// "1200" to "1,200" with `separator`; keeps a leading minus.
pub(crate) fn group_thousands(digits: &str, separator: char) -> String {
    let (sign, body) = digits
        .strip_prefix('-')
        .map_or(("", digits), |rest| ("-", rest));
    let mut out = String::with_capacity(body.len() + body.len() / 3 * separator.len_utf8());
    for (i, c) in body.chars().enumerate() {
        if i > 0 && (body.len() - i) % 3 == 0 {
            out.push(separator);
        }
        out.push(c);
    }
    format!("{sign}{out}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_follow_the_locale() {
        let en = Locale::ENGLISH;
        assert_eq!(en.number(1234.56, 1), "1234.6");
        assert_eq!(en.grouped(1234.56, 1), "1,234.6");
        assert_eq!(en.grouped(-4500.0, 0), "-4,500");
        assert_eq!(en.number(-0.04, 1), "0.0");
        let de = Locale {
            numbers: NumberStyle::Comma,
            ..Locale::ENGLISH
        };
        assert_eq!(de.number(12.5, 1), "12,5");
        assert_eq!(de.grouped(1_234_567.8, 1), "1.234.567,8");
        let fr = Locale::PSEUDO;
        assert_eq!(fr.grouped(12_345.0, 0), "12\u{202F}345");
        assert_eq!(fr.number(0.25, 2), "0,25");
        assert_eq!(Locale::default(), Locale::ENGLISH);
    }

    #[test]
    fn thousands_group_by_three() {
        assert_eq!(group_thousands("1200", ','), "1,200");
        assert_eq!(group_thousands("999", ','), "999");
        assert_eq!(group_thousands("1234567", ','), "1,234,567");
        assert_eq!(group_thousands("-4500", '.'), "-4.500");
    }
}
