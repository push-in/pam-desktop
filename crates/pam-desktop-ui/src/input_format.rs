//! Input masks: `InputFormat` patterns (phone numbers, dates, document ids)
//! and currency, applied by the host while the user types.

/// How an input reformats its text.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Format {
    #[default]
    None,
    /// Digits fill the `slot` characters of `pattern`; everything else is literal.
    Pattern { pattern: String, slot: char },
    /// Digits shift in from the right: `1234` with two decimals is `12.34`.
    Currency { prefix: String, suffix: String, decimals: u8, locale: String },
}

fn digits(text: &str) -> Vec<char> {
    text.chars().filter(|character| character.is_numeric() && character.is_ascii_digit()).collect()
}

fn count_digits(text: &str) -> usize {
    digits(text).len()
}

/// Thousands and decimal separators of a BCP-47 locale.
fn separators(locale: &str) -> (&'static str, &'static str) {
    let language = locale.split(['-', '_', '.']).next().unwrap_or("").to_ascii_lowercase();
    match language.as_str() {
        "pt" | "de" | "es" | "it" | "nl" | "id" | "tr" | "da" | "el" | "hr" | "sl" | "ro" | "vi" => (".", ","),
        "fr" | "nb" | "no" | "sv" | "fi" | "cs" | "sk" | "pl" | "ru" | "uk" | "bg" | "hu" => ("\u{202f}", ","),
        "ch" => ("'", "."),
        _ => (",", "."),
    }
}

/// The locale of the process environment (`LC_ALL`, `LC_NUMERIC`, `LANG`).
#[must_use]
pub fn system_locale() -> String {
    ["LC_ALL", "LC_NUMERIC", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty() && value != "C" && value != "POSIX")
        .unwrap_or_else(|| String::from("en-US"))
}

impl Format {
    /// Formats a complete value (also applied to values coming from PHP).
    #[must_use]
    pub fn format(&self, value: &str) -> String {
        match self {
            Self::None => value.to_owned(),
            Self::Pattern { pattern, slot } => {
                let digits = digits(value);
                if digits.is_empty() {
                    return String::new();
                }
                if pattern.is_empty() {
                    return value.to_owned();
                }
                let mut output = String::with_capacity(pattern.len());
                let mut used = 0;
                for character in pattern.chars() {
                    if character == *slot {
                        if used == digits.len() {
                            break;
                        }
                        output.push(digits[used]);
                        used += 1;
                    } else if used == 0 || used < digits.len() {
                        // Leading literals always show; trailing ones only
                        // while more digits follow.
                        output.push(character);
                    }
                }
                output
            }
            Self::Currency { prefix, suffix, decimals, locale } => {
                let digits: Vec<char> = digits(value).into_iter().take(36).collect();
                if digits.is_empty() {
                    return String::new();
                }
                let decimals = usize::from(*decimals);
                // Normalize to ASCII digits and drop leading zeros of the integer part.
                let mut ascii: String = digits.iter().filter_map(|digit| digit.to_digit(10)).map(|digit| char::from(b'0' + digit as u8)).collect();
                while ascii.len() <= decimals {
                    ascii.insert(0, '0');
                }
                let (whole, fraction) = ascii.split_at(ascii.len() - decimals);
                let whole = whole.trim_start_matches('0');
                let whole = if whole.is_empty() { "0" } else { whole };
                let (thousands, decimal) = separators(if locale.is_empty() { "en-US" } else { locale });
                let mut grouped = String::new();
                for (index, character) in whole.chars().enumerate() {
                    if index > 0 && (whole.len() - index) % 3 == 0 {
                        grouped.push_str(thousands);
                    }
                    grouped.push(character);
                }
                let mut output = String::with_capacity(prefix.len() + grouped.len() + decimals + suffix.len() + 1);
                output.push_str(prefix);
                output.push_str(&grouped);
                if decimals > 0 {
                    output.push_str(decimal);
                    output.push_str(fraction);
                }
                output.push_str(suffix);
                output
            }
        }
    }

    /// Reformats after a user edit. `caret` is the caret offset in
    /// characters within `proposed`; returns the text and the new caret.
    #[must_use]
    pub fn edit(&self, previous: &str, proposed: &str, caret: usize) -> (String, usize) {
        if matches!(self, Self::None) {
            return (proposed.to_owned(), caret);
        }
        let mut proposed: Vec<char> = proposed.chars().collect();
        let mut caret = caret.min(proposed.len());
        let deletion = proposed.len() < previous.chars().count();
        // Deleting a literal removes the digit before it: otherwise the mask
        // would put the literal straight back and the caret would be stuck.
        if deletion && count_digits(&proposed.iter().collect::<String>()) == count_digits(previous) {
            let before: Vec<usize> = proposed[..caret]
                .iter()
                .enumerate()
                .filter(|(_, character)| character.is_ascii_digit())
                .map(|(index, _)| index)
                .collect();
            if let Some(index) = before.last() {
                proposed.remove(*index);
                caret -= 1;
            }
        }
        let digits_before = proposed[..caret.min(proposed.len())].iter().filter(|character| character.is_ascii_digit()).count();
        let proposed: String = proposed.into_iter().collect();
        let formatted = self.format(&proposed);
        let length = formatted.chars().count();
        let caret = match self {
            Self::Currency { .. } => length,
            _ => {
                if digits_before == 0 {
                    0
                } else {
                    // Just after the n-th digit of the formatted text.
                    let mut seen = 0;
                    let mut position = length;
                    for (index, character) in formatted.chars().enumerate() {
                        if character.is_ascii_digit() {
                            seen += 1;
                            if seen == digits_before {
                                position = index + 1;
                                break;
                            }
                        }
                    }
                    position
                }
            }
        };
        (formatted, caret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone() -> Format {
        Format::Pattern { pattern: "(##) #####-####".into(), slot: '#' }
    }

    #[test]
    fn pattern_fills_slots_and_hides_trailing_literals() {
        let format = phone();
        assert_eq!(format.format(""), "");
        assert_eq!(format.format("1"), "(1");
        assert_eq!(format.format("11"), "(11");
        assert_eq!(format.format("119"), "(11) 9");
        assert_eq!(format.format("11987654321"), "(11) 98765-4321");
        assert_eq!(format.format("11987654321999"), "(11) 98765-4321");
        assert_eq!(format.format("abc"), "");
    }

    #[test]
    fn currency_shifts_digits_in_from_the_right() {
        let brl = Format::Currency { prefix: "R$ ".into(), suffix: String::new(), decimals: 2, locale: "pt-BR".into() };
        assert_eq!(brl.format("1"), "R$ 0,01");
        assert_eq!(brl.format("12"), "R$ 0,12");
        assert_eq!(brl.format("123456"), "R$ 1.234,56");
        assert_eq!(brl.format("R$ 1.234,567"), "R$ 12.345,67");
        let usd = Format::Currency { prefix: "$".into(), suffix: " USD".into(), decimals: 0, locale: "en-US".into() };
        assert_eq!(usd.format("1234567"), "$1,234,567 USD");
        assert_eq!(usd.format(""), "");
    }

    #[test]
    fn typing_keeps_the_caret_after_the_typed_digit() {
        let format = phone();
        // "(11" + typing "9" at the end.
        assert_eq!(format.edit("(11", "(119", 4), ("(11) 9".to_owned(), 6));
        // Inserting a digit in the middle keeps the caret after it.
        assert_eq!(format.edit("(11) 9", "(121) 9", 3), ("(12) 19".to_owned(), 3));
    }

    #[test]
    fn deleting_a_literal_deletes_the_digit_before_it() {
        let format = phone();
        // Backspace over the space in "(11) 9|" positioned after ") ".
        let (text, caret) = format.edit("(11) 9", "(11)9", 4);
        assert_eq!(text, "(19");
        assert_eq!(caret, 2);
        // Deleting a digit behaves normally.
        assert_eq!(format.edit("(11) 9", "(11) ", 5), ("(11".to_owned(), 3));
    }

    #[test]
    fn currency_caret_goes_to_the_end() {
        let format = Format::Currency { prefix: String::new(), suffix: String::new(), decimals: 2, locale: "en-US".into() };
        assert_eq!(format.edit("0.12", "0.123", 5), ("1.23".to_owned(), 4));
    }
}
