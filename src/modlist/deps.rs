use crate::util::workshop_url;

/// Workshop app ids that are applications, not mods. Steam lists them as
/// required items on some pages, but they must never be repacked as mods.
pub const NON_MOD_APP_IDS: [&str; 2] = ["107410", "228800"];

/// True when `id` is a non-mod Steam application id.
pub fn is_non_mod_app_id(id: &str) -> bool {
    NON_MOD_APP_IDS.contains(&id)
}

/// Parse a Workshop page's "Required Items" section.
/// Returns Vec<(id, name)>. Empty if the section is absent.
pub fn parse_required_items(html: &str) -> Vec<(String, String)> {
    let document = scraper::Html::parse_document(html);

    // Steam renders required items in a div with id="RequiredItems"
    // Each item is a link: <a href="...?id=NNN">Name</a>
    let Ok(required_selector) = scraper::Selector::parse("#RequiredItems") else {
        return Vec::new();
    };
    let Some(required_section) = document.select(&required_selector).next() else {
        return Vec::new();
    };
    let Ok(link_selector) = scraper::Selector::parse("a") else {
        return Vec::new();
    };

    let mut deps = Vec::new();
    for link in required_section.select(&link_selector) {
        let href = link.value().attr("href").unwrap_or("");
        // Extract id=NNN from the href
        let id = href
            .split("?id=")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .unwrap_or("");
        if id.is_empty() || is_non_mod_app_id(id) {
            continue;
        }
        let name = link.text().collect::<String>().trim().to_string();
        if !name.is_empty() {
            deps.push((id.to_string(), name));
        }
    }
    deps
}

/// Fetch a Workshop item's page and return its required dependencies as
/// Vec<(id, name)>. Returns empty on network error or if no deps exist.
pub(crate) fn fetch_workshop_dependencies(workshop_id: &str) -> Vec<(String, String)> {
    let url = workshop_url(workshop_id);
    // A stalled connection must not hang the CLI indefinitely.
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Warning: failed to build HTTP client: {}", e);
            return Vec::new();
        }
    };
    let body = match client.get(&url).send() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "Warning: failed to fetch Workshop page for {}: {}",
                workshop_id, e
            );
            return Vec::new();
        }
    };
    let html = match body.text() {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "Warning: failed to read response for {}: {}",
                workshop_id, e
            );
            return Vec::new();
        }
    };
    parse_required_items(&html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_required_items_real_structure() {
        // Mirrors the current Steam HTML: links wrap a div.requiredItem
        let html = r#"<div id="rightContents">
<div class="requiredItemsContainer" id="RequiredItems">
<a href="https://steamcommunity.com/workshop/filedetails/?id=2262006564" target="_blank" data-subscribed="0">
<div class="requiredItem">cTab 1erGTD</div>
</a>
<a href="https://steamcommunity.com/workshop/filedetails/?id=2853828143" target="_blank" data-subscribed="0">
<div class="requiredItem">Better CAS Environment (BCE)</div>
</a>
</div>
</div>"#;
        let deps = parse_required_items(html);
        assert_eq!(
            deps,
            vec![
                ("2262006564".to_string(), "cTab 1erGTD".to_string()),
                (
                    "2853828143".to_string(),
                    "Better CAS Environment (BCE)".to_string()
                ),
            ]
        );
    }

    #[test]
    fn parse_required_items_no_section() {
        assert!(parse_required_items("<html><body>no deps</body></html>").is_empty());
    }

    #[test]
    fn parse_required_items_filters_non_mod_app_ids() {
        let html = r#"<div id="RequiredItems">
<a href="https://steamcommunity.com/sharedfiles/filedetails/?id=107410"><div class="requiredItem">Arma 3</div></a>
<a href="https://steamcommunity.com/sharedfiles/filedetails/?id=228800"><div class="requiredItem">DayZ</div></a>
<a href="https://steamcommunity.com/sharedfiles/filedetails/?id=450814997"><div class="requiredItem">CBA_A3</div></a>
</div>"#;
        assert_eq!(
            parse_required_items(html),
            vec![("450814997".to_string(), "CBA_A3".to_string())]
        );
    }
}
