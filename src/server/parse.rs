//! Parse `sftp` batch output and `meta.cpp` files.
//!
//! A listing block starts at the `Remote working directory:` line that
//! `pwd` prints and holds the following `ls -l` entries.

use crate::pbo::key_of;
use crate::util::extract_quoted;

const PWD_PREFIX: &str = "Remote working directory:";

/// One `ls -l` line. The file name keeps its spaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
}

/// One directory listing: the path from `pwd` and its entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub path: String,
    pub entries: Vec<Entry>,
}

/// The two fields this program reads from `meta.cpp`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetaCpp {
    pub publishedid: Option<String>,
    pub name: Option<String>,
}

/// Split the client output into listing blocks. Echo lines and blank lines
/// are ignored, and so are lines that are not `ls -l` entries.
pub fn parse_blocks(output: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut current: Option<Block> = None;

    for raw in output.lines() {
        let line = raw.trim_start();
        if let Some(path) = line.strip_prefix(PWD_PREFIX) {
            if let Some(block) = current.take() {
                blocks.push(block);
            }
            current = Some(Block {
                path: path.trim().to_string(),
                entries: Vec::new(),
            });
            continue;
        }
        let Some(block) = current.as_mut() else {
            continue;
        };
        if line.starts_with("sftp>") || line.trim().is_empty() {
            continue;
        }
        if let Some(entry) = parse_entry_line(line.trim_end()) {
            block.entries.push(entry);
        }
    }
    if let Some(block) = current {
        blocks.push(block);
    }
    blocks
}

/// Find the block for one remote path. A server that reports a chrooted
/// path is accepted when its path ends with the requested one.
pub fn find_block<'a>(blocks: &'a [Block], path: &str) -> Option<&'a Block> {
    blocks
        .iter()
        .find(|block| same_remote_path(&block.path, path))
}

fn same_remote_path(block: &str, target: &str) -> bool {
    let block = block.trim_end_matches('/');
    let target = target.trim_end_matches('/');
    if block == target {
        return true;
    }
    target != "/" && !target.is_empty() && block.ends_with(target)
}

/// Parse one long listing line:
/// `perms links owner group size month day time-or-year name...`.
fn parse_entry_line(line: &str) -> Option<Entry> {
    let mut fields = line.split_whitespace();
    let perms = fields.next()?;
    let kind = *perms.as_bytes().first()?;
    if !matches!(kind, b'-' | b'd' | b'l') || perms.len() != 10 {
        return None;
    }
    if !perms.as_bytes()[1..]
        .iter()
        .all(|b| matches!(b, b'r' | b'w' | b'x' | b'-' | b's' | b'S' | b't' | b'T'))
    {
        return None;
    }

    let _links = fields.next()?;
    let _owner = fields.next()?;
    let _group = fields.next()?;
    let size: u64 = fields.next()?.parse().ok()?;
    let _month = fields.next()?;
    let _day = fields.next()?;
    let _time = fields.next()?;
    let rest: Vec<&str> = fields.collect();
    if rest.is_empty() {
        return None;
    }

    let is_symlink = kind == b'l';
    let raw_name = rest.join(" ");
    let name = if is_symlink {
        raw_name
            .split(" -> ")
            .next()
            .unwrap_or(&raw_name)
            .to_string()
    } else {
        raw_name
    };
    if name == "." || name == ".." {
        return None;
    }
    Some(Entry {
        name,
        is_dir: kind == b'd',
        is_symlink,
        size,
    })
}

/// Read `publishedid` and `name` from `meta.cpp` text. The value may be
/// quoted or bare, and may end with a semicolon.
pub fn parse_meta_cpp(content: &str) -> MetaCpp {
    let mut meta = MetaCpp::default();
    for line in content.lines() {
        let line = line.trim();
        let key = key_of(line);
        let value = value_of(line);
        match (key, value) {
            (Some("publishedid"), Some(value)) => meta.publishedid = Some(value),
            (Some("name"), Some(value)) => meta.name = Some(value),
            _ => {}
        }
    }
    meta
}

/// A quoted value first, then a bare value with the trailing `;` removed.
fn value_of(line: &str) -> Option<String> {
    if let Some(quoted) = extract_quoted(line) {
        return Some(quoted);
    }
    let (_, raw) = line.split_once('=')?;
    let value = raw
        .trim()
        .trim_end_matches(';')
        .trim()
        .trim_matches('"')
        .trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = "\
sftp> cd \"/srv/arma\"
sftp> pwd
Remote working directory: /srv/arma
sftp> ls -l
drwxrwxr-x    3 1000     1000         4096 Nov 20 10:00 @450814997
-rw-rw-r--    1 1000     1000         1234 Nov 20 10:00 server.cfg
sftp> cd \"/srv/arma/@450814997/addons\"
sftp> pwd
Remote working directory: /srv/arma/@450814997/addons
sftp> ls -l
total 16
-rw-rw-r--    1 1000     1000         5000 Nov 20 10:00 Big Mod.pbo
-rw-rw-r--    1 1000     1000           10 Nov 20 10:00 Big Mod.pbo.cba_3.20.0.bisign
drwxrwxr-x    2 1000     1000         4096 Nov 20 10:00 nested
lrwxrwxrwx    1 1000     1000            5 Nov 20 10:00 link.pbo -> /etc/passwd
sftp> cd \"/srv/arma/@450814997/addons/nested\"
sftp> pwd
Remote working directory: /srv/arma/@450814997/addons/nested
sftp> ls -l
";

    #[test]
    fn parse_blocks_splits_on_the_pwd_line() {
        let blocks = parse_blocks(LISTING);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].path, "/srv/arma");
        assert_eq!(blocks[1].path, "/srv/arma/@450814997/addons");
        assert!(blocks[2].entries.is_empty());
    }

    #[test]
    fn parse_blocks_reads_sizes_dirs_and_spaces_in_names() {
        let blocks = parse_blocks(LISTING);
        let entries = &blocks[1].entries;
        let pbo = entries.iter().find(|e| e.name == "Big Mod.pbo").unwrap();
        assert_eq!(pbo.size, 5000);
        assert!(!pbo.is_dir);
        assert!(!pbo.is_symlink);
        let nested = entries.iter().find(|e| e.name == "nested").unwrap();
        assert!(nested.is_dir);
        let total = entries.iter().find(|e| e.name.starts_with("total"));
        assert!(total.is_none(), "a total line is not an entry");
    }

    #[test]
    fn parse_blocks_marks_a_symlink_and_strips_its_target() {
        let blocks = parse_blocks(LISTING);
        let link = blocks[1]
            .entries
            .iter()
            .find(|e| e.name == "link.pbo")
            .unwrap();
        assert!(link.is_symlink);
        assert!(!link.is_dir);
    }

    #[test]
    fn parse_blocks_ignores_dot_entries_and_handles_an_empty_listing() {
        let text = "\
Remote working directory: /srv/empty
sftp> ls -l
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 .
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 ..
";
        let blocks = parse_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].entries.is_empty());
    }

    #[test]
    fn find_block_matches_the_exact_path_and_a_chrooted_suffix() {
        let blocks = parse_blocks(LISTING);
        assert!(find_block(&blocks, "/srv/arma").is_some());

        let chrooted = parse_blocks("Remote working directory: /jail/srv/arma\nsftp> ls -l\n");
        assert!(find_block(&chrooted, "/srv/arma").is_some());
        assert!(find_block(&blocks, "/srv/arma/@1/addons").is_none());
    }

    #[test]
    fn parse_meta_cpp_reads_the_server_format() {
        let meta = parse_meta_cpp(
            "protocol = 1;\npublishedid = 450814997;\nname = \"CBA_A3\";\ntimestamp = 5;\n",
        );
        assert_eq!(meta.publishedid.as_deref(), Some("450814997"));
        assert_eq!(meta.name.as_deref(), Some("CBA_A3"));
    }

    #[test]
    fn parse_meta_cpp_reads_a_quoted_id() {
        let meta =
            parse_meta_cpp("publishedid = \"1234567890\"\nname = \"Right\"\nnamespace = \"x\"\n");
        assert_eq!(meta.publishedid.as_deref(), Some("1234567890"));
        assert_eq!(meta.name.as_deref(), Some("Right"));
    }

    #[test]
    fn parse_meta_cpp_without_publishedid_is_none() {
        let meta = parse_meta_cpp("protocol = 1;\nname = \"No Id\";\n");
        assert!(meta.publishedid.is_none());
        assert_eq!(meta.name.as_deref(), Some("No Id"));
    }
}
