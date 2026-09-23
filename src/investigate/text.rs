//! Search-term tokenisation and title-matching helpers.

/// Format a count for humans: 47770 → "47.8k", 1541757 → "1.5M".
pub(crate) fn human_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Split a camelCase or underscore_separated string into tokens.
/// "zuluslicksters" → ["zuluslicksters"] (no split possible)
/// "zulu_custom_slicksters" → ["zulu", "custom", "slicksters"]
/// "tfl_headgear" → ["tfl", "headgear"]
pub(crate) fn split_camel_or_underscore(s: &str) -> Vec<&str> {
    // If underscores are present, split on them
    if s.contains('_') {
        return s.split('_').filter(|t| !t.is_empty()).collect();
    }
    // Otherwise, try camelCase split
    let mut tokens = Vec::new();
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if c.is_uppercase() && i > start {
            let token = &s[start..i];
            if !token.is_empty() {
                tokens.push(token);
            }
            start = i;
        }
    }
    let last = &s[start..];
    if !last.is_empty() {
        tokens.push(last);
    }
    // If we only got one token, return the whole thing
    if tokens.len() <= 1 {
        vec![s]
    } else {
        tokens
    }
}

/// True when `term` appears in `title` as a meaningful match.
/// Short terms (< 5 chars) must match as a whole word or as a suffix
/// of a longer word. Suffix allows compound acronyms: "nvg" in
/// "GPNVG-18" is a real match. Prefix matches are rejected: "sty"
/// in "style" is noise, not a match.
pub(crate) fn title_contains_term(title_lower: &str, term_lower: &str) -> bool {
    if term_lower.len() < 5 {
        title_lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w == term_lower || (w.len() > term_lower.len() && w.ends_with(term_lower)))
    } else {
        title_lower.contains(term_lower)
    }
}

/// Word-level overlap score between a search term and a title.
/// Splits both into words, checks how many search-term words appear
/// in the title. More discriminative than character-level matching
/// because it avoids false positives from common letters.
///
/// "aceax" → ["aceax"] → if "ACEAX" in title → 1.0
/// "zulu_custom_slicksters" → ["zulu","custom","slicksters"] → "A2 Declassified: Fireteam Zulu" has "zulu" → 1/3 = 0.33
/// "usasoc_backpacks" → ["usasoc","backpacks"] → "121 USASOC Sniper Rifles Pack" has "usasoc" → 1/2 = 0.5
///
/// Short words (< 5 chars) must match exactly. "sty" is not "style".
pub(crate) fn word_overlap_score(term_lower: &str, title_lower: &str) -> f64 {
    let term_words: Vec<&str> = split_camel_or_underscore(term_lower);
    if term_words.is_empty() {
        return 0.0;
    }

    // Split title on non-alphanumeric boundaries
    let title_words: Vec<&str> = title_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    let hits = term_words
        .iter()
        .filter(|tw| {
            if tw.len() < 3 {
                return false;
            }
            title_words.iter().any(|t| {
                if tw.len() < 5 {
                    // Short words must match exactly or as a suffix of a
                    // longer word. "gpnvg" contains "nvg" (compound), but
                    // "style" must not count as "sty" (prefix noise).
                    *t == **tw || (t.len() > tw.len() && t.ends_with(*tw))
                } else {
                    *t == **tw || t.contains(*tw)
                }
            })
        })
        .count();

    hits as f64 / term_words.len() as f64
}

/// Generate multiple search variations from a single search term.
/// "sps_blackhornet" → ["sps blackhornet", "blackhornet", "sps"]
/// "ZuluCustomSlicksters" → ["zulu custom slicksters", "slicksters",
///   "zulu", "custom", "zulu custom", "custom slicksters"]
/// The idea: one term may not match the Workshop title, but a
/// substring or reordering might. We try all of them and merge.
pub(crate) fn search_variation_terms(term: &str) -> Vec<String> {
    let tokens = split_camel_or_underscore(term);
    let mut variations = Vec::new();

    if tokens.len() <= 1 {
        // Single token: try it as-is
        variations.push(term.to_string());
        return variations;
    }

    // 1. All tokens joined with spaces (full term)
    let full = tokens.join(" ");
    if full != term {
        variations.push(full);
    }

    // 2. Individual tokens (>= 3 chars to avoid noise)
    for t in &tokens {
        if t.len() >= 3 {
            variations.push(t.to_string());
        }
    }

    // 3. Pairs of adjacent tokens
    for w in tokens.windows(2) {
        variations.push(w.join(" "));
    }

    // 4. Last token first (reversal — "blackhornet sps")
    if tokens.len() >= 2 {
        let mut rev: Vec<&str> = tokens.iter().rev().copied().collect();
        rev.truncate(3); // cap at 3 tokens
        variations.push(rev.join(" "));
    }

    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    variations.retain(|v| seen.insert(v.clone()));
    variations
}
