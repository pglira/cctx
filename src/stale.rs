use crate::model::{Flag, Group, Item};
use crate::util::{age_days, expand, home};
use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;

fn path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?:^|[\s`'"(\[=])((?:~|/)[A-Za-z0-9._@+\-/]*[A-Za-z0-9_\-/])"#).unwrap()
    })
}

/// Absolute and `~/` paths in `content` that do not exist, as (line number, path).
/// Single-segment paths such as `/help` are slash commands and do not count.
/// Paths followed by a placeholder character (`<`, `{`, `*`, `$`) do not count.
pub fn missing_paths(content: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let h = home();
    for (i, line) in content.lines().enumerate() {
        for cap in path_re().captures_iter(line) {
            let m = cap.get(1).unwrap();
            let token_start = line[..m.start()]
                .rfind(char::is_whitespace)
                .map_or(0, |i| i + 1);
            let token_end = line[m.end()..]
                .find(char::is_whitespace)
                .map_or(line.len(), |i| m.end() + i);
            if line[token_start..token_end].contains("://") {
                continue;
            }
            let next = line[m.end()..].chars().next();
            if matches!(next, Some('<' | '{' | '*' | '$' | '[' | '%')) {
                continue;
            }
            let raw = m.as_str().trim_end_matches(['/', '.']);
            if raw.contains("...") {
                continue;
            }
            let segs = raw.trim_start_matches(['~', '/']);
            if raw.starts_with('/') && !segs.contains('/') {
                continue;
            }
            if raw.starts_with('~') && !raw.starts_with("~/") {
                continue;
            }
            if segs.is_empty() {
                continue;
            }
            let p = expand(raw, &h);
            if !p.exists() && seen.insert(raw.to_string()) {
                out.push((i + 1, raw.to_string()));
            }
        }
    }
    out
}

pub fn annotate(it: &mut Item, stale_days: u64) {
    if it.read_only {
        return;
    }
    let ages = matches!(it.group, Group::Instructions | Group::Memory);
    if let Some(d) = age_days(it.mtime).filter(|_| ages) {
        if d >= stale_days {
            it.flags.push(Flag::Old { days: d });
        }
    }
    for (line, path) in missing_paths(&it.content) {
        it.flags.push(Flag::MissingPath { line, path });
    }
}
