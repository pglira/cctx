use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Configuration directory of Claude Code: `$CLAUDE_CONFIG_DIR` or `~/.claude`.
pub fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".claude"))
}

/// Replace the home directory prefix with `~`.
pub fn tilde(p: &Path) -> String {
    let h = home();
    match p.strip_prefix(&h) {
        Ok(rest) if h != Path::new("/") => {
            if rest.as_os_str().is_empty() {
                "~".into()
            } else {
                format!("~/{}", rest.display())
            }
        }
        _ => p.display().to_string(),
    }
}

/// Resolve `~/x`, absolute, and relative paths. Relative paths resolve against `base`.
pub fn expand(s: &str, base: &Path) -> PathBuf {
    if s == "~" {
        home()
    } else if let Some(rest) = s.strip_prefix("~/") {
        home().join(rest)
    } else if s.starts_with('/') {
        PathBuf::from(s)
    } else {
        base.join(s)
    }
}

/// Encode a path the way Claude Code names its per-project data directories:
/// every character that is not ASCII alphanumeric becomes `-`.
pub fn encode(p: &Path) -> String {
    encode_str(&p.to_string_lossy())
}

pub fn encode_str(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Rough token estimate: four characters per token.
pub fn tokens(s: &str) -> usize {
    s.chars().count().div_ceil(4)
}

pub fn fmt_tokens(n: usize) -> String {
    if n < 1000 {
        n.to_string()
    } else if n < 100_000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        format!("{}k", n / 1000)
    }
}

pub fn fmt_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n}B")
    } else if n < 1024 * 1024 {
        format!("{:.1}K", n as f64 / 1024.0)
    } else {
        format!("{:.1}M", n as f64 / 1024.0 / 1024.0)
    }
}

pub fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

pub fn age_days(t: Option<SystemTime>) -> Option<u64> {
    let d = SystemTime::now().duration_since(t?).ok()?;
    Some(d.as_secs() / 86_400)
}

pub fn fmt_age(days: Option<u64>) -> String {
    match days {
        None => "-".into(),
        Some(d) if d < 14 => format!("{d}d"),
        Some(d) if d < 60 => format!("{}w", d / 7),
        Some(d) if d < 730 => format!("{}mo", d / 30),
        Some(d) => format!("{}y", d / 365),
    }
}

pub fn read(p: &Path) -> Option<String> {
    std::fs::read(p)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Parse simple YAML frontmatter `key: value` pairs. Handles quoted values and
/// folded (`>`) or literal (`|`) block values. Does not need valid YAML.
pub fn frontmatter(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(rest) = content.strip_prefix("---") else {
        return out;
    };
    let rest = rest.trim_start_matches('\r');
    let Some(rest) = rest.strip_prefix('\n') else {
        return out;
    };
    let mut lines = rest.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim_end() == "---" {
            break;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let mut v = v.trim().to_string();
        if v.is_empty() || v.starts_with('>') || v.starts_with('|') {
            let mut parts = Vec::new();
            while let Some(next) = lines.peek() {
                if next.starts_with(' ') || next.starts_with('\t') {
                    parts.push(next.trim().to_string());
                    lines.next();
                } else {
                    break;
                }
            }
            v = parts.join(" ");
        }
        let v = v
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string();
        out.push((k.trim().to_string(), v));
    }
    out
}

pub fn fm_get(fm: &[(String, String)], key: &str) -> Option<String> {
    fm.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

/// Content after the frontmatter block.
pub fn body(content: &str) -> &str {
    if let Some(rest) = content.strip_prefix("---") {
        if let Some(i) = rest.find("\n---") {
            let after = &rest[i + 4..];
            return after.split_once('\n').map(|x| x.1).unwrap_or("");
        }
    }
    content
}

pub fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else if w == 0 {
        String::new()
    } else {
        let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
