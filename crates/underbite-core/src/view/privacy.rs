//! The words of the Privacy page: what stays on the phone, what leaves it, what the person
//! sends, and the rounding of the place sent to data providers. Plain fact, the same in every
//! voice: every text is a [`SafetyKey`], so none has a Luna variant.

use crate::view::strings::{SafetyKey, Strings};

/// The finest rounding the app sends, and the choices offered, finest first.
pub const PRIVACY_DECIMALS: [u8; 3] = [2, 1, 0];

/// One section of the page: a title and its paragraphs, plain sentences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivacySection {
    pub title: String,
    pub paragraphs: Vec<String>,
}

/// One choice of the rounding setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivacyChoice {
    /// The decimals of latitude and longitude the choice sends.
    pub decimals: u8,
    /// "2 decimals, about 1 km".
    pub label: String,
}

/// Everything the Privacy page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivacyWords {
    /// What stays on the device, what leaves it and what the person chooses to send.
    pub sections: Vec<PrivacySection>,
    /// The title of the rounding setting.
    pub rounding_title: String,
    /// The current rounding, as the settings hold it.
    pub rounding_current: String,
    /// The choices, finest first.
    pub rounding_choices: Vec<PrivacyChoice>,
    /// What the setting means.
    pub rounding_footer: String,
    /// The label of a current setting that no choice covers.
    pub rounding_too_fine: String,
}

impl Strings {
    /// The label of a rounding of `decimals`: a choice's label, or the bare count for one the
    /// page does not offer.
    pub fn privacy_rounding(self, decimals: u8) -> String {
        match decimals {
            2 => self.safety(SafetyKey::PrivacyRound2).to_owned(),
            1 => self.safety(SafetyKey::PrivacyRound1).to_owned(),
            0 => self.safety(SafetyKey::PrivacyRound0).to_owned(),
            n => self.safety_fmt(SafetyKey::PrivacyRoundOther, &[("n", &n.to_string())]),
        }
    }
}

/// The Privacy page's words for a settings' `coordinate_precision` of `decimals`.
pub fn privacy_words(strings: Strings, decimals: u8) -> PrivacyWords {
    let section = |title: SafetyKey, paragraphs: &[SafetyKey]| PrivacySection {
        title: strings.safety(title).to_owned(),
        paragraphs: paragraphs
            .iter()
            .map(|key| strings.safety(*key).to_owned())
            .collect(),
    };
    PrivacyWords {
        sections: vec![
            section(
                SafetyKey::PrivacyStaysTitle,
                &[
                    SafetyKey::PrivacyStaysData,
                    SafetyKey::PrivacyStaysLocation,
                    SafetyKey::PrivacyStaysExact,
                    SafetyKey::PrivacyStaysKeys,
                ],
            ),
            section(
                SafetyKey::PrivacyLeavesTitle,
                &[
                    SafetyKey::PrivacyLeavesIntro,
                    SafetyKey::PrivacyLeavesPlace,
                    SafetyKey::PrivacyLeavesArea,
                    SafetyKey::PrivacyLeavesSearch,
                    SafetyKey::PrivacyLeavesServices,
                ],
            ),
            section(
                SafetyKey::PrivacySendTitle,
                &[
                    SafetyKey::PrivacySendShares,
                    SafetyKey::PrivacySendICloud,
                    SafetyKey::PrivacySendExports,
                ],
            ),
        ],
        rounding_title: strings.safety(SafetyKey::PrivacyRoundingTitle).to_owned(),
        rounding_current: strings.privacy_rounding(decimals),
        rounding_choices: PRIVACY_DECIMALS
            .iter()
            .map(|&d| PrivacyChoice {
                decimals: d,
                label: strings.privacy_rounding(d),
            })
            .collect(),
        rounding_footer: strings.safety(SafetyKey::PrivacyRoundingFooter).to_owned(),
        rounding_too_fine: strings.safety(SafetyKey::PrivacyTooFine).to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Voice;
    use crate::view::locale::Locale;

    #[test]
    fn the_current_rounding_follows_the_settings() {
        let s = Strings::new(Voice::Plain);
        let now = |d| privacy_words(s, d).rounding_current;
        assert_eq!(now(2), "2 decimals, about 1 km");
        assert_eq!(now(1), "1 decimal, about 10 km");
        assert_eq!(now(0), "Whole degrees, about 100 km");
        assert_eq!(now(4), "4 decimals");
    }

    #[test]
    fn every_choice_has_a_label_and_the_choices_are_two_one_none() {
        let words = privacy_words(Strings::new(Voice::Plain), 2);
        let decimals: Vec<u8> = words.rounding_choices.iter().map(|c| c.decimals).collect();
        assert_eq!(decimals, [2, 1, 0]);
        for choice in &words.rounding_choices {
            assert_ne!(choice.label, "");
            assert_eq!(
                choice.label,
                privacy_words(Strings::new(Voice::Plain), choice.decimals).rounding_current
            );
        }
        assert_ne!(words.rounding_title, "");
        assert_ne!(words.rounding_footer, "");
        assert_eq!(
            words.rounding_too_fine,
            "Finer than allowed: choose another"
        );
    }

    #[test]
    fn the_page_has_three_sections_of_plain_sentences() {
        let words = privacy_words(Strings::new(Voice::Plain), 2);
        let titles: Vec<&str> = words.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "What stays on your device",
                "What leaves the device",
                "What you choose to send"
            ]
        );
        for text in words.sections.iter().flat_map(|s| &s.paragraphs) {
            assert!(text.ends_with('.'), "{text}");
            assert!(!text.contains(['*', '`', '#', '[']), "{text}");
        }
    }

    #[test]
    fn privacy_words_are_the_same_in_every_voice() {
        for decimals in [0, 1, 2, 3] {
            assert_eq!(
                privacy_words(Strings::new(Voice::Plain), decimals),
                privacy_words(Strings::new(Voice::Luna), decimals)
            );
        }
    }

    #[test]
    fn privacy_words_are_translated_in_the_pseudo_locale() {
        for voice in [Voice::Plain, Voice::Luna] {
            let words = privacy_words(Strings::localized(voice, Locale::PSEUDO), 2);
            let mut texts = vec![
                words.rounding_title,
                words.rounding_current,
                words.rounding_footer,
                words.rounding_too_fine,
            ];
            texts.extend(words.rounding_choices.into_iter().map(|c| c.label));
            for s in words.sections {
                texts.push(s.title);
                texts.extend(s.paragraphs);
            }
            let english = crate::view::pseudo_tests::english_texts(&texts, &[]);
            assert!(english.is_empty(), "{}", english.join("\n"));
        }
    }
}
