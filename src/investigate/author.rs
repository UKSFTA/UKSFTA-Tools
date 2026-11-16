//! The creator name on a Workshop item page.
//!
//! The browse `creator` field is a SteamID64, so it can never equal the
//! `author` in a PBO config. The item page names the uploader as text in
//! `creatorsBlock`, which is the only place the two can agree.

use super::http::{fetch_with_retry, Endpoint, Http, RateLimiter};
use super::identity::IdentityEvidence;
use super::normalise::normalise;

/// Marks the panel that holds one block per creator.
const CREATORS_BLOCK: &str = "creatorsBlock";

/// Marks the div that holds one creator name and its presence text.
const CREATOR_CONTENT: &str = "friendBlockContent";

/// Attributes that mark the element carrying the item's own description.
const DESCRIPTION_MARKERS: [&str; 2] = [
    "id=\"highlightContent\"",
    "class=\"workshopItemDescription\"",
];

/// The most description text to keep, in characters.
const DESCRIPTION_LIMIT: usize = 20_000;

/// The parts of an item page the identity path reads.
pub(crate) struct ItemPage {
    pub author: Option<String>,
    pub description: String,
}

/// Parse the creator name and the item description from one item page.
pub(crate) fn parse_item_page(html: &str) -> ItemPage {
    ItemPage {
        author: parse_author(html),
        description: parse_description(html),
    }
}

/// Fetch and parse the creator name and description for one item id.
pub(crate) fn fetch_item_page(
    http: &dyn Http,
    limiter: &mut RateLimiter,
    id: &str,
) -> Option<ItemPage> {
    let url = format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}");
    let body = fetch_with_retry(http, limiter, Endpoint::SharedFiles, &url).ok()?;
    Some(parse_item_page(&body))
}

/// The first creator name on an item page.
///
/// The name is the text of the first `friendBlockContent` div, before the
/// `<br>` that separates it from the presence line. Tags are stripped and
/// the five common entities are decoded.
fn parse_author(html: &str) -> Option<String> {
    let block = &html[html.find(CREATORS_BLOCK)? + CREATORS_BLOCK.len()..];
    let content = &block[block.find(CREATOR_CONTENT)? + CREATOR_CONTENT.len()..];
    // Skip the rest of the div's own tag, then stop at the line break.
    let inner = &content[content.find('>')? + 1..];
    let text = match inner.find("<br") {
        Some(end) => &inner[..end],
        None => inner,
    };
    let name = decode_entities(&strip_tags(text));
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// The item's own description text, with tags stripped, entities decoded,
/// whitespace collapsed, and the length capped at [`DESCRIPTION_LIMIT`]
/// characters. A page without the description element yields an empty
/// string.
fn parse_description(html: &str) -> String {
    let Some(open_tag_end) = description_open_tag_end(html) else {
        return String::new();
    };
    let inner = &html[open_tag_end..];
    let inner = &inner[..description_close(inner)];
    let text = decode_entities(&strip_tags(inner));
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(DESCRIPTION_LIMIT).collect()
}

/// The byte offset just past the opening tag of the description element.
/// Whichever marker appears first in the page wins, so the element the
/// page actually uses is the one read.
fn description_open_tag_end(html: &str) -> Option<usize> {
    let marker_at = DESCRIPTION_MARKERS
        .iter()
        .filter_map(|marker| html.find(marker))
        .min()?;
    Some(html[marker_at..].find('>')? + marker_at + 1)
}

/// The byte offset of the `</div>` that closes the element whose content
/// starts at `inner`. Nested divs are counted, so a wrapper inside the
/// description does not end it early.
fn description_close(inner: &str) -> usize {
    let mut depth = 1usize;
    let mut cursor = 0usize;
    while let Some(offset) = inner[cursor..].find('<') {
        let at = cursor + offset;
        let rest = &inner[at..];
        if rest
            .get(..4)
            .is_some_and(|tag| tag.eq_ignore_ascii_case("<div"))
        {
            depth += 1;
            cursor = at + 4;
        } else if rest
            .get(..5)
            .is_some_and(|tag| tag.eq_ignore_ascii_case("</div"))
        {
            depth -= 1;
            if depth == 0 {
                return at;
            }
            cursor = at + 5;
        } else {
            cursor = at + 1;
        }
    }
    inner.len()
}

/// Whether the PBO config names an author that could ever match.
///
/// A `$STR_` token is a localisation key, not a name, and a key or author
/// shorter than three characters cannot identify anyone.
pub(crate) fn has_usable_author(evidence: &IdentityEvidence) -> bool {
    usable_authors(evidence).next().is_some()
}

/// Whether an item page's creator names the PBO's own author.
///
/// Only a `CfgMods`/`CfgPatches` author counts. Normalisation drops the
/// brackets, so `[ADE]` matches the uploader `ADE Team` without them.
pub(crate) fn author_matches(evidence: &IdentityEvidence, author: &str) -> bool {
    let item = normalise(author).squashed;
    usable_authors(evidence).any(|pbo| item.contains(&normalise(pbo).squashed))
}

fn usable_authors(evidence: &IdentityEvidence) -> impl Iterator<Item = &str> {
    [
        evidence.cfgmods_author.as_deref(),
        evidence.cfgpatches_author.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|author| !author.starts_with("$STR_"))
    .filter(|author| normalise(author).squashed.chars().count() >= 3)
}

/// Remove every tag, keeping the text between them.
fn strip_tags(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => kept.push(c),
            _ => {}
        }
    }
    kept
}

/// Decode the five entities a creator name is likely to carry. `&amp;`
/// runs last, so `&amp;lt;` stays text.
fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The creator panel of an item page, cut down to what the parser reads.
    fn item_page(creators: &[(&str, &str)]) -> String {
        let blocks: String = creators
            .iter()
            .map(|(id, name)| {
                format!(
                    r#"<div class="friendBlock persona offline" data-miniprofile="{id}">
                        <div class="friendBlockContent">
                            {name}<br>
                            <span class="friendSmallText">Offline</span>
                        </div>
                    </div>"#
                )
            })
            .collect();
        format!(r#"<div class="rightDetailsBlock"><div class="creatorsBlock">{blocks}</div></div>"#)
    }

    fn page_with_description(creators: &[(&str, &str)], description: &str) -> String {
        format!(
            r#"{page}<div class="workshopItemDescription" id="highlightContent">{description}</div>"#,
            page = item_page(creators)
        )
    }

    fn evidence(cfgmods_author: Option<&str>, cfgpatches_author: Option<&str>) -> IdentityEvidence {
        IdentityEvidence {
            pbo: "ade_core.pbo".to_string(),
            cfgmods_author: cfgmods_author.map(str::to_string),
            cfgpatches_author: cfgpatches_author.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn parse_item_author_reads_first_creator() {
        let html = item_page(&[("57858904", "MadMax"), ("96844772", "N. Brody")]);

        let author = parse_item_page(&html).author;

        assert_eq!(author.as_deref(), Some("MadMax"));
    }

    #[test]
    fn parse_item_author_decodes_entities() {
        let html = item_page(&[("1", "O&amp;T &#39;Bob&#39; &lt;Ade&gt;")]);

        let author = parse_item_page(&html).author;

        assert_eq!(author.as_deref(), Some("O&T 'Bob' <Ade>"));
    }

    #[test]
    fn parse_item_author_missing_block_is_none() {
        assert_eq!(
            parse_item_page("<div class=\"panel\">No creators</div>").author,
            None
        );
        assert_eq!(parse_item_page("").author, None);
        // A creator block with no name in it names nobody.
        assert_eq!(parse_item_page(&item_page(&[("1", "")])).author, None);
    }

    #[test]
    fn parse_item_page_reads_author_and_description() {
        let html = page_with_description(
            &[("57858904", "MadMax")],
            "A <b>trauma</b> kit &amp; pouch
             <br>
             for medics",
        );

        let page = parse_item_page(&html);

        assert_eq!(page.author.as_deref(), Some("MadMax"));
        assert_eq!(page.description, "A trauma kit & pouch for medics");
    }

    #[test]
    fn parse_item_page_missing_description_is_empty() {
        let page = parse_item_page(&item_page(&[("1", "MadMax")]));

        assert_eq!(page.description, "");
        assert_eq!(page.author.as_deref(), Some("MadMax"));
    }

    #[test]
    fn parse_item_page_truncates_long_description() {
        let text = format!("{}ééé", "a".repeat(DESCRIPTION_LIMIT - 1));

        let page = parse_item_page(&page_with_description(&[], &text));

        assert_eq!(page.description.chars().count(), DESCRIPTION_LIMIT);
        assert!(page.description.ends_with('é'));
    }

    #[test]
    fn author_matches_ignores_str_token() {
        let item = evidence(None, Some("$STR_ADE_Author"));

        assert!(!has_usable_author(&item));
        assert!(!author_matches(&item, "$STR_ADE_Author"));
    }

    #[test]
    fn author_matches_bracket_tag() {
        let item = evidence(Some("[ADE]"), None);

        assert!(author_matches(&item, "ADE"));
        assert!(author_matches(&item, "ADE Team"));
    }

    #[test]
    fn author_matches_rejects_unrelated_name() {
        let item = evidence(Some("[ADE] SovjetPwr"), None);

        assert!(!author_matches(&item, "MadMax"));
    }
}
