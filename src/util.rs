use std::fs;
use std::path::{Path, PathBuf};

pub fn extract_quoted(line: &str) -> Option<String> {
    let chars = line.chars();
    let mut in_quote = false;
    let mut value = String::new();

    for c in chars {
        if c == '"' {
            if in_quote {
                return Some(value);
            }
            in_quote = true;
        } else if in_quote {
            value.push(c);
        }
    }
    None
}

pub fn extract_quoted_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars = line.chars();
    let mut in_quote = false;
    let mut value = String::new();

    for c in chars {
        if c == '"' {
            if in_quote {
                tokens.push(std::mem::take(&mut value));
                in_quote = false;
            } else {
                in_quote = true;
            }
        } else if in_quote {
            value.push(c);
        }
    }
    tokens
}

/// Build the Steam Workshop page URL for a mod ID.
pub fn workshop_url(id: &str) -> String {
    format!(
        "https://steamcommunity.com/sharedfiles/filedetails/?id={}",
        id
    )
}

/// Escape a string for use inside a TOML basic string (double-quoted).
/// Mod names come from untrusted sources, so quotes, backslashes and
/// control characters must not corrupt the generated mod_sources.txt.
pub fn toml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7F => {
                out.push_str(&format!("\\u{:04X}", c as u32))
            }
            c => out.push(c),
        }
    }
    out
}

/// Truncate a string to at most `max` characters, never splitting a
/// multi-byte character. Returns the longest prefix that fits.
pub fn truncate_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Strip terminal control characters from remote-derived text. Escape
/// sequences injected by a scraped title or changelog could otherwise
/// rewrite the terminal title or hide output. Keeps `\t`, `\n`, `\r`.
pub fn sanitize_for_terminal(s: &str) -> String {
    s.chars()
        .filter(|c| {
            let n = *c as u32;
            !(n <= 0x08
                || n == 0x0B
                || n == 0x0C
                || (0x0E..=0x1F).contains(&n)
                || n == 0x7F
                || (0x80..=0x9F).contains(&n))
        })
        .collect()
}

pub fn extract_id(line: &str) -> Option<String> {
    // Match 8+ digit IDs in URLs or bare. The `id=` key is accepted only
    // when it starts a query segment, so `steamid=` and `valid=` do not
    // match.
    for word in line.split_whitespace() {
        if let Some(pos) = word.find('?') {
            for segment in word[pos + 1..].split('&') {
                if let Some(rest) = segment.strip_prefix("id=") {
                    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if digits.len() >= 8 {
                        return Some(digits);
                    }
                }
            }
        }
        // Bare ID
        if word.chars().all(|c| c.is_ascii_digit()) && word.len() >= 8 {
            return Some(word.to_string());
        }
    }
    None
}

pub fn extract_tag(line: &str) -> Option<String> {
    if let Some(pos) = line.find('#') {
        let tag = line[pos + 1..].trim().to_string();
        if !tag.is_empty() {
            return Some(tag);
        }
    }
    None
}

// --- PBO sync ---

pub fn find_pbos(dir: &Path) -> Vec<PathBuf> {
    let mut pbos = Vec::new();
    if !dir.exists() {
        return pbos;
    }
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry),
            Err(e) => {
                eprintln!("Warning: skipped unreadable path: {}", e);
                None
            }
        })
    {
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                if ext.to_string_lossy().to_lowercase() == "pbo" {
                    pbos.push(entry.path().to_path_buf());
                }
            }
        }
    }
    pbos
}

/// SHA256 of a file's contents, hex-encoded. None on read failure.
pub fn file_sha256(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Some(digest.iter().map(|b| format!("{:02x}", b)).collect())
}

// --- Terminal output helpers ---

/// ANSI colour codes, applied only when stdout is a terminal.
/// Piped output stays plain so logs and CI are not polluted.
pub const C_RESET: &str = "\x1b[0m";
pub const C_GREEN: &str = "\x1b[32m";
pub const C_YELLOW: &str = "\x1b[33m";
pub const C_RED: &str = "\x1b[31m";
pub const C_DIM: &str = "\x1b[2m";
pub const C_BOLD: &str = "\x1b[1m";

/// True when stdout is a terminal, so colour codes are safe to use.
pub fn stdout_is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// Wrap text in a colour, or return it unchanged when colour is off.
pub fn paint(use_colour: bool, code: &str, text: &str) -> String {
    if use_colour {
        format!("{}{}{}", code, text, C_RESET)
    } else {
        text.to_string()
    }
}

/// Format a byte count for humans: 1500000 -> "1.5 MB".
/// Uses decimal units, matching how Steam reports Workshop sizes.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

/// Percent-encode a query string for the Workshop browse URL.
pub fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_keeps_ascii_prefix() {
        assert_eq!(truncate_chars("abcdef", 3), "abc");
        assert_eq!(truncate_chars("abc", 3), "abc");
        assert_eq!(truncate_chars("abc", 10), "abc");
    }

    #[test]
    fn truncate_chars_does_not_split_multibyte() {
        // "é" is two bytes; slicing at byte 3 would panic without
        // char_indices. The cut lands on a character boundary.
        assert_eq!(truncate_chars("aébc", 2), "aé");
        assert_eq!(truncate_chars("日本語", 2), "日本");
    }

    #[test]
    fn sanitize_for_terminal_strips_escape_sequence() {
        assert_eq!(sanitize_for_terminal("\x1b]0;x\x07title"), "]0;xtitle");
    }

    #[test]
    fn sanitize_for_terminal_keeps_tab_newline_carriage_return() {
        assert_eq!(sanitize_for_terminal("a\tb\nc\rd"), "a\tb\nc\rd");
    }

    #[test]
    fn sanitize_for_terminal_strips_del_and_c1() {
        assert_eq!(sanitize_for_terminal("a\u{7f}b"), "ab");
        assert_eq!(sanitize_for_terminal("a\u{9b}b"), "ab");
    }

    #[test]
    fn extract_id_requires_query_segment_boundary() {
        assert_eq!(extract_id("steamid=12345678"), None);
        assert_eq!(extract_id("valid=1234567890"), None);
        assert_eq!(extract_id("https://x/?id=450814997").unwrap(), "450814997");
        assert_eq!(
            extract_id("https://x/?a=1&id=450814997").unwrap(),
            "450814997"
        );
        assert_eq!(extract_id("https://x/?steamid=450814997"), None);
    }

    #[test]
    fn toml_escape_escapes_del() {
        assert_eq!(toml_escape("\u{7f}"), "\\u007F");
    }

    #[test]
    fn find_pbos_finds_pbos_recursively() {
        let dir = std::env::temp_dir().join("uksfta-find-pbos");
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("a.pbo"), b"a").unwrap();
        fs::write(dir.join("nested").join("b.PBO"), b"b").unwrap();
        fs::write(dir.join("nested").join("c.txt"), b"c").unwrap();
        let found = find_pbos(&dir);
        assert_eq!(found.len(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_pbos_missing_dir_returns_empty() {
        assert!(find_pbos(Path::new("/nonexistent/uksfta/find-pbos")).is_empty());
    }

    #[test]
    fn extract_id_sharedfiles_url() {
        assert_eq!(
            extract_id("https://steamcommunity.com/sharedfiles/filedetails/?id=450814997").unwrap(),
            "450814997"
        );
    }

    #[test]
    fn extract_id_workshop_url() {
        // Current Steam pages link deps via /workshop/filedetails/
        assert_eq!(
            extract_id("https://steamcommunity.com/workshop/filedetails/?id=2262006564").unwrap(),
            "2262006564"
        );
    }

    #[test]
    fn extract_id_bare() {
        assert_eq!(extract_id("450814997").unwrap(), "450814997");
    }

    #[test]
    fn extract_id_with_name_comment() {
        assert_eq!(extract_id("450814997 # CBA_A3").unwrap(), "450814997");
    }

    #[test]
    fn extract_id_rejects_short_numbers() {
        // Steam IDs are 8+ digits; 7-digit numbers must not match
        assert_eq!(extract_id("1234567"), None);
        assert_eq!(extract_id("id=1234567"), None);
    }

    #[test]
    fn extract_id_rejects_non_id() {
        assert_eq!(extract_id("no id here"), None);
        assert_eq!(extract_id(""), None);
    }

    #[test]
    fn toml_escape_handles_quotes_backslashes_and_controls() {
        assert_eq!(toml_escape("plain"), "plain");
        assert_eq!(toml_escape("a\"b"), "a\\\"b");
        assert_eq!(toml_escape("a\\b"), "a\\\\b");
        assert_eq!(toml_escape("a\nb"), "a\\nb");
        assert_eq!(toml_escape("a\tb"), "a\\tb");
    }

    #[test]
    fn file_sha256_matches_known_value() {
        let dir = std::env::temp_dir().join("uksfta-sha-test");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("data.bin");
        fs::write(&f, b"hello world").unwrap();
        // sha256 of "hello world"
        assert_eq!(
            file_sha256(&f).unwrap(),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn file_sha256_missing_file_returns_none() {
        assert_eq!(file_sha256(Path::new("/nonexistent/uksfta/test.pbo")), None);
    }

    #[test]
    fn urlencode_encodes_spaces_and_symbols() {
        assert_eq!(urlencode("TFL Headgear"), "TFL%20Headgear");
        assert_eq!(urlencode("a/b&c"), "a%2Fb%26c");
        assert_eq!(urlencode("ACE"), "ACE");
    }

    #[test]
    fn human_size_scales_to_largest_unit() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1000), "1.0 KB");
        assert_eq!(human_size(1_500_000), "1.5 MB");
        assert_eq!(human_size(2_000_000_000), "2.0 GB");
    }
}
