//! Workshop changelog scraping.

/// Scrape the Workshop changelog page for a mod and extract mod-name
/// references. The changelog often names source mods that a pack
/// repacked, e.g. "Updated ACE3 to 3.16.0" or "Added FFAA MOD".
pub(crate) fn scrape_changelog(
    workshop_id: &str,
    client: &reqwest::blocking::Client,
) -> Vec<String> {
    let url = format!(
        "https://steamcommunity.com/sharedfiles/filedetails/changelog/{}",
        workshop_id
    );
    let html = match client.get(&url).send().ok().and_then(|r| r.text().ok()) {
        Some(h) => h,
        None => return Vec::new(),
    };

    let mut names = Vec::new();
    // Changelog entries are in <div class="entry"> blocks. Extract text
    // content and look for known mod name patterns.
    for entry in html.split("<div class=\"entry\">").skip(1) {
        let text_end = entry.find("</div>").unwrap_or(entry.len());
        let text = &entry[..text_end];
        // Strip HTML tags to get plain text
        let mut in_tag = false;
        let plain: String = text
            .chars()
            .filter(|&c| {
                if c == '<' {
                    in_tag = true;
                    false
                } else if c == '>' {
                    in_tag = false;
                    false
                } else {
                    !in_tag
                }
            })
            .collect();
        let plain = plain.trim();
        if plain.len() > 5 {
            names.push(plain.to_string());
        }
    }
    names
}
