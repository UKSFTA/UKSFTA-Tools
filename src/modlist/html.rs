use crate::error::UksftaError;
use crate::util::workshop_url;
use std::path::Path;

const MODLIST_TEMPLATE_HEADER: &str = "\
<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<html>\n\
<!--Exported with uksfta: https://github.com/UKSFTA/UKSFTA-Tools-->\n\
  <head>\n\
    <meta name=\"arma:Type\" content=\"preset\" />\n\
    <meta name=\"arma:PresetName\" content=\"{{PRESET_NAME}}\" />\n\
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
    <h1>Arma 3  - Preset <strong>{{PRESET_NAME}}</strong></h1>\n\
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

/// Build the full modlist HTML for a preset name and an ordered mod list.
/// The preset name is HTML-escaped before it enters the header.
pub fn preset_html(mods: &[(String, String)], preset_name: &str) -> String {
    let escaped_preset = html_escape::encode_safe(preset_name);
    let mut html = MODLIST_TEMPLATE_HEADER.replace("{{PRESET_NAME}}", &escaped_preset);
    for (id, name) in mods {
        html.push_str(&modlist_row_html(id, name));
    }
    html.push_str(MODLIST_TEMPLATE_FOOTER);
    html
}

/// Write the modlist HTML for `mods` with the given preset name.
pub fn generate_preset(
    mods: &[(String, String)],
    path: &Path,
    preset_name: &str,
) -> Result<(), UksftaError> {
    let html = preset_html(mods, preset_name);
    crate::atomic::write_atomic(path, html.as_bytes())?;
    println!("Modlist written to {}", path.display());
    Ok(())
}

pub fn generate_modlist(missing: &[(String, String)], path: &Path) -> Result<(), UksftaError> {
    if missing.is_empty() {
        return Ok(());
    }

    generate_preset(missing, path, "UKSFTA Missing Mods")
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

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("uksfta-html-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn generate_modlist_writes_nothing_when_empty() {
        let dir = temp_dir("empty");
        let path = dir.join("missing-mods.html");
        generate_modlist(&[], &path).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generate_modlist_keeps_missing_mods_preset_and_title() {
        let dir = temp_dir("missing");
        let path = dir.join("missing-mods.html");
        generate_modlist(&[("450814997".to_string(), "CBA_A3".to_string())], &path).unwrap();
        let html = std::fs::read_to_string(&path).unwrap();
        assert!(html.contains("content=\"UKSFTA Missing Mods\""));
        assert!(html.contains("<strong>UKSFTA Missing Mods</strong>"));
        assert!(html.contains("<title>Arma 3</title>"));
        assert!(html.contains("CBA_A3"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preset_html_escapes_preset_name() {
        let html = preset_html(&[], "<b>Pack</b>");
        assert!(html.contains("content=\"&lt;b&gt;Pack&lt;&#x2F;b&gt;\""));
        assert!(html.contains("<strong>&lt;b&gt;Pack&lt;&#x2F;b&gt;</strong>"));
        assert!(!html.contains("<b>Pack</b>"));
    }
}
