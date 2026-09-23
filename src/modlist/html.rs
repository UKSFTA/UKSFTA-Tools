use crate::error::UksftaError;
use crate::util::workshop_url;
use std::path::Path;

const MODLIST_TEMPLATE_HEADER: &str = "\
<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<html>\n\
<!--Exported with uksfta: https://github.com/UKSFTA/UKSFTA-Tools-->\n\
  <head>\n\
    <meta name=\"arma:Type\" content=\"preset\" />\n\
    <meta name=\"arma:PresetName\" content=\"UKSFTA Missing Mods\" />\n\
    <meta name=\"generator\" content=\"uksfta\" />\n\
    <title>Arma 3</title>\n\
    <link href=\"https://fonts.googleapis.com/css?family=Roboto\" rel=\"stylesheet\" type=\"text/css\" />\n\
    <style>\n\
body {\n\
\tmargin: 0;\n\
\tpadding: 0;\n\
\tcolor: #fff;\n\
\tbackground: #000;\n\
}\n\
\n\
body, th, td {\n\
\tfont: 95%/1.3 Roboto, Segoe UI, Tahoma, Arial, Helvetica, sans-serif;\n\
}\n\
\n\
td {\n\
    padding: 3px 30px 3px 0;\n\
}\n\
\n\
h1 {\n\
    padding: 20px 20px 0 20px;\n\
    color: white;\n\
    font-weight: 200;\n\
    font-family: segoe ui;\n\
    font-size: 3em;\n\
    margin: 0;\n\
}\n\
\n\
em {\n\
    font-variant: italic;\n\
    color:silver;\n\
}\n\
\n\
.before-list {\n\
    padding: 5px 20px 10px 20px;\n\
}\n\
\n\
.mod-list {\n\
    background: #222222;\n\
    padding: 20px;\n\
}\n\
\n\
.dlc-list {\n\
    background: #222222;\n\
    padding: 20px;\n\
}\n\
\n\
.footer {\n\
    padding: 20px;\n\
    color:gray;\n\
}\n\
\n\
.whups {\n\
    color:gray;\n\
}\n\
\n\
a {\n\
    color: #D18F21;\n\
    text-decoration: underline;\n\
}\n\
\n\
a:hover {\n\
    color:#F1AF41;\n\
    text-decoration: none;\n\
}\n\
\n\
.from-steam {\n\
    color: #449EBD;\n\
}\n\
.from-local {\n\
    color: gray;\n\
}\n\
\n\
</style>\n\
  </head>\n\
  <body>\n\
    <h1>Arma 3  - Preset <strong>UKSFTA Missing Mods</strong></h1>\n\
    <p class=\"before-list\">\n\
      <em>To import this preset, drag this file onto the Launcher window. Or click the MODS tab, then PRESET in the top right, then IMPORT at the bottom, and finally select this file.</em>\n\
    </p>\n\
    <div class=\"mod-list\">\n\
      <table>\n";

const MODLIST_TEMPLATE_FOOTER: &str = "\
      </table>\n\
    </div>\n\
    <div class=\"dlc-list\">\n\
      <table />\n\
    </div>\n\
    <div class=\"footer\">\n\
      <span>Created by uksfta.</span>\n\
    </div>\n\
  </body>\n\
</html>\n";

/// Build a single mod row for the modlist HTML. Names are HTML-escaped.
pub fn modlist_row_html(id: &str, name: &str) -> String {
    let url = workshop_url(id);
    // Mod names come from Steam Workshop pages (attacker-controlled), so
    // escape all HTML metacharacters, not just ampersands.
    let escaped_name = html_escape::encode_safe(name);
    format!(
        "        <tr data-type=\"ModContainer\">\n\
              <td data-type=\"DisplayName\">{}</td>\n\
              <td>\n\
                <span class=\"from-steam\">Steam</span>\n\
              </td>\n\
              <td>\n\
                <a href=\"{}\" data-type=\"Link\">{}</a>\n\
              </td>\n\
            </tr>\n",
        escaped_name, url, url
    )
}

pub fn generate_modlist(missing: &[(String, String)], path: &Path) -> Result<(), UksftaError> {
    if missing.is_empty() {
        return Ok(());
    }

    let mut html = String::from(MODLIST_TEMPLATE_HEADER);
    for (id, name) in missing {
        html.push_str(&modlist_row_html(id, name));
    }
    html.push_str(MODLIST_TEMPLATE_FOOTER);

    crate::atomic::write_atomic(path, html.as_bytes())?;
    println!("Modlist written to {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modlist_row_html_escapes_ampersand() {
        let row = modlist_row_html("1234567890", "O&T Warfighters");
        assert!(row.contains("O&amp;T Warfighters"));
        assert!(!row.contains("O&T Warfighters"));
        assert!(row.contains("id=1234567890"));
        assert!(row.contains("data-type=\"ModContainer\""));
    }

    #[test]
    fn modlist_row_html_escapes_html_metacharacters() {
        // A malicious Workshop name must not inject markup into the HTML.
        let row = modlist_row_html("1234567890", "<script>alert(1)</script>");
        assert!(!row.contains("<script>"));
        assert!(row.contains("&lt;script&gt;"));
        let quoted = modlist_row_html("1234567890", "Mod \"quoted\"");
        assert!(quoted.contains("Mod &quot;quoted&quot;"));
        assert!(!quoted.contains("Mod \"quoted\""));
    }
}
