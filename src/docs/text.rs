//! Sorting helpers for user-facing mod names. Case and common accents are
//! folded so `Çukurca` sorts with `C`, not after `Z`.

/// A comparison key: lower case with common Latin diacritics folded to ASCII.
pub fn sort_key(name: &str) -> String {
    let mut out = String::new();
    for character in name.chars().flat_map(char::to_lowercase) {
        match fold(character) {
            Some(folded) => out.push_str(folded),
            None => out.push(character),
        }
    }
    out
}

fn fold(character: char) -> Option<&'static str> {
    Some(match character {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'č' => "c",
        'ď' | 'đ' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ş' | 'š' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_and_accents() {
        assert_eq!(sort_key("Çukurca"), "cukurca");
        assert_eq!(sort_key("Æther"), "aether");
        assert_eq!(sort_key("Straße"), "strasse");
        assert_eq!(sort_key("Éléphant"), "elephant");
    }

    #[test]
    fn sorts_accented_names_with_their_base_letter() {
        let mut names = ["Zulu", "Çukurca", "Alpha", "kerama"];
        names.sort_by_key(|name| sort_key(name));
        assert_eq!(names, ["Alpha", "Çukurca", "kerama", "Zulu"]);
    }
}
