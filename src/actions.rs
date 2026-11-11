use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::path::Path;

/// Move a file or directory to the trash of the desktop.
pub fn trash(p: &Path) -> Result<()> {
    trash::delete(p).with_context(|| format!("cannot move {} to trash", p.display()))
}

/// Remove the lines of a MEMORY.md index that link to `file_name`. Returns the count.
pub fn unlink_from_index(index: &Path, file_name: &str) -> Result<usize> {
    let Ok(text) = std::fs::read_to_string(index) else {
        return Ok(0);
    };
    let needle = format!("]({file_name})");
    let kept: Vec<&str> = text.lines().filter(|l| !l.contains(&needle)).collect();
    let removed = text.lines().count() - kept.len();
    if removed > 0 {
        let mut out = kept.join("\n");
        if text.ends_with('\n') {
            out.push('\n');
        }
        std::fs::write(index, out)?;
    }
    Ok(removed)
}

/// Set (`off = true`) or clear the `skillOverrides` entry of a skill in a settings file.
pub fn set_skill_off(settings: &Path, key: &str, off: bool) -> Result<()> {
    let text = std::fs::read_to_string(settings).unwrap_or_else(|_| "{}".into());
    let mut v: Value = serde_json::from_str(&text).context("settings file is not valid JSON")?;
    let root = v.as_object_mut().context("settings root is not an object")?;
    let entry = root
        .entry("skillOverrides")
        .or_insert_with(|| Value::Object(Map::new()));
    let m = entry.as_object_mut().context("skillOverrides is not an object")?;
    if off {
        m.insert(key.to_string(), Value::String("off".into()));
    } else {
        m.remove(key);
    }
    if m.is_empty() {
        root.remove("skillOverrides");
    }
    let mut out = serde_json::to_string_pretty(&v)?;
    out.push('\n');
    write_atomic(settings, &out)
}

/// Replace the content of `path`. The previous content goes to the trash first.
pub fn replace_with_backup(path: &Path, new: &str) -> Result<()> {
    let name = path.file_name().context("no file name")?.to_string_lossy();
    let backup = path.with_file_name(format!("{name}.before-review"));
    std::fs::copy(path, &backup)?;
    trash(&backup)?;
    write_atomic(path, new)
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let tmp = path.with_extension("cctx-tmp");
    std::fs::write(&tmp, text)?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}
