//! Text normalisation shared by phrase matching and title matching.
//!
//! One function, one shape: apostrophes removed before splitting,
//! lowercase, Latin diacritics folded to ASCII through a static table,
//! every non-alphanumeric run turned into a single space. The result
//! carries the token list, the tokens squashed together, and the tokens
//! as one string.

/// A normalised string: word tokens, the tokens concatenated, and the
/// tokens separated by single spaces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Normalised {
    pub tokens: Vec<String>,
    pub squashed: String,
    pub text: String,
}

/// Lowercase ASCII foldings for the Latin-1 Supplement and Latin
/// Extended-A letters. Input is lowercased first, so only the lowercase
/// code points are listed. Characters outside the table are kept.
const FOLD_TABLE: &[(char, &str)] = &[
    ('à', "a"),
    ('á', "a"),
    ('â', "a"),
    ('ã', "a"),
    ('ä', "a"),
    ('å', "a"),
    ('æ', "ae"),
    ('ç', "c"),
    ('è', "e"),
    ('é', "e"),
    ('ê', "e"),
    ('ë', "e"),
    ('ì', "i"),
    ('í', "i"),
    ('î', "i"),
    ('ï', "i"),
    ('ð', "d"),
    ('ñ', "n"),
    ('ò', "o"),
    ('ó', "o"),
    ('ô', "o"),
    ('õ', "o"),
    ('ö', "o"),
    ('ø', "o"),
    ('ù', "u"),
    ('ú', "u"),
    ('û', "u"),
    ('ü', "u"),
    ('ý', "y"),
    ('þ', "th"),
    ('ÿ', "y"),
    ('ā', "a"),
    ('ă', "a"),
    ('ą', "a"),
    ('ć', "c"),
    ('ĉ', "c"),
    ('ċ', "c"),
    ('č', "c"),
    ('ď', "d"),
    ('đ', "d"),
    ('ē', "e"),
    ('ĕ', "e"),
    ('ė', "e"),
    ('ę', "e"),
    ('ě', "e"),
    ('ĝ', "g"),
    ('ğ', "g"),
    ('ġ', "g"),
    ('ģ', "g"),
    ('ĥ', "h"),
    ('ħ', "h"),
    ('ĩ', "i"),
    ('ī', "i"),
    ('ĭ', "i"),
    ('į', "i"),
    ('ı', "i"),
    ('ĳ', "ij"),
    ('ĵ', "j"),
    ('ķ', "k"),
    ('ĸ', "k"),
    ('ĺ', "l"),
    ('ļ', "l"),
    ('ľ', "l"),
    ('ŀ', "l"),
    ('ł', "l"),
    ('ń', "n"),
    ('ņ', "n"),
    ('ň', "n"),
    ('ŉ', "n"),
    ('ŋ', "n"),
    ('ō', "o"),
    ('ŏ', "o"),
    ('ő', "o"),
    ('œ', "oe"),
    ('ŕ', "r"),
    ('ŗ', "r"),
    ('ř', "r"),
    ('ś', "s"),
    ('ŝ', "s"),
    ('ş', "s"),
    ('š', "s"),
    ('ţ', "t"),
    ('ť', "t"),
    ('ŧ', "t"),
    ('ũ', "u"),
    ('ū', "u"),
    ('ŭ', "u"),
    ('ů', "u"),
    ('ű', "u"),
    ('ų', "u"),
    ('ŵ', "w"),
    ('ŷ', "y"),
    ('ź', "z"),
    ('ż', "z"),
    ('ž', "z"),
    ('ſ', "s"),
];

/// Fold the diacritics of an already lowercased string to ASCII.
fn fold_diacritics(s: &str) -> String {
    let mut folded = String::with_capacity(s.len());
    for c in s.chars() {
        match FOLD_TABLE.iter().find(|(from, _)| *from == c) {
            Some((_, to)) => folded.push_str(to),
            None => folded.push(c),
        }
    }
    folded
}

/// Normalise a phrase for matching.
pub(crate) fn normalise(s: &str) -> Normalised {
    let joined: String = s
        .chars()
        .filter(|c| !matches!(c, '\'' | '\u{2019}'))
        .collect();
    let folded = fold_diacritics(&joined.to_lowercase());

    let mut tokens: Vec<String> = Vec::new();
    let mut token = String::new();
    for c in folded.chars() {
        if c.is_alphanumeric() {
            token.push(c);
        } else if !token.is_empty() {
            tokens.push(std::mem::take(&mut token));
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }

    let squashed = tokens.concat();
    let text = tokens.join(" ");
    Normalised {
        tokens,
        squashed,
        text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_removes_apostrophes() {
        let normalised = normalise("Jason's GPNVG-18");

        assert_eq!(normalised.tokens, ["jasons", "gpnvg", "18"]);
        assert_eq!(normalised.squashed, "jasonsgpnvg18");
    }

    #[test]
    fn normalise_maps_hyphen_and_underscore() {
        assert_eq!(normalise("GPNVG-18_NVG").tokens, ["gpnvg", "18", "nvg"]);
    }

    #[test]
    fn normalise_folds_latin_diacritics() {
        assert_eq!(normalise("Café Élite").tokens, ["cafe", "elite"]);
        assert_eq!(normalise("Škoda Ørsted").tokens, ["skoda", "orsted"]);
    }

    #[test]
    fn normalise_keeps_non_latin_and_never_panics() {
        assert_eq!(normalise("日本語 Название").tokens, ["日本語", "название"]);
        assert_eq!(normalise(""), Normalised::default());
        assert_eq!(normalise("--- !!!").tokens, Vec::<String>::new());
        assert_eq!(normalise("’quote’").tokens, ["quote"]);
    }
}
