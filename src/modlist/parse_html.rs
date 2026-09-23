use crate::util::extract_id;

/// Parse an Arma 3 launcher modlist HTML document.
/// Returns (steam mods as (id, name), local mod names without a Workshop ID).
pub fn parse_modlist_html(content: &str) -> (Vec<(String, String)>, Vec<String>) {
    let document = scraper::Html::parse_document(content);
    let row_selector = scraper::Selector::parse("tr[data-type=\"ModContainer\"]").unwrap();
    let name_selector = scraper::Selector::parse("td[data-type=\"DisplayName\"]").unwrap();
    let link_selector = scraper::Selector::parse("a[data-type=\"Link\"]").unwrap();

    let mut found: Vec<(String, String)> = Vec::new(); // (id, name)
    let mut local_mods: Vec<String> = Vec::new();

    for row in document.select(&row_selector) {
        let name = row
            .select(&name_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let href = row
            .select(&link_selector)
            .next()
            .and_then(|el| el.value().attr("href"))
            .unwrap_or("");

        match extract_id(href) {
            Some(id) => found.push((id, name)),
            None => {
                if !name.is_empty() {
                    local_mods.push(name);
                }
            }
        }
    }
    (found, local_mods)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODLIST_SAMPLE: &str = r#"<html><body><div class="mod-list"><table>
<tr data-type="ModContainer">
<td data-type="DisplayName">CBA_A3</td>
<td><span class="from-steam">Steam</span></td>
<td><a href="https://steamcommunity.com/sharedfiles/filedetails/?id=450814997" data-type="Link">URL</a></td>
</tr>
<tr data-type="ModContainer">
<td data-type="DisplayName">O&amp;T Warfighters</td>
<td><span class="from-steam">Steam</span></td>
<td><a href="https://steamcommunity.com/sharedfiles/filedetails/?id=1234567890" data-type="Link">URL</a></td>
</tr>
<tr data-type="ModContainer">
<td data-type="DisplayName">Local Custom Mod</td>
<td><span class="from-local">Local</span></td>
<td></td>
</tr>
</table></div></body></html>"#;

    #[test]
    fn parse_modlist_html_extracts_steam_and_local() {
        let (found, local) = parse_modlist_html(MODLIST_SAMPLE);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], ("450814997".to_string(), "CBA_A3".to_string()));
        // HTML entities are decoded by the parser
        assert_eq!(
            found[1],
            ("1234567890".to_string(), "O&T Warfighters".to_string())
        );
        assert_eq!(local, vec!["Local Custom Mod".to_string()]);
    }

    #[test]
    fn parse_modlist_html_empty() {
        let (found, local) = parse_modlist_html("<html><body></body></html>");
        assert!(found.is_empty());
        assert!(local.is_empty());
    }

    #[test]
    fn parse_modlist_html_not_a_modlist() {
        let (found, local) = parse_modlist_html("this is not html");
        assert!(found.is_empty());
        assert!(local.is_empty());
    }
}
