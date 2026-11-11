use crate::util::*;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One per-project data directory below `<claude dir>/projects`.
#[derive(Clone, Debug)]
pub struct Project {
    pub dir: PathBuf,
    pub name: String,
    pub real: Option<PathBuf>,
    pub exists: bool,
    pub mem_files: usize,
    pub mem_bytes: u64,
    pub sessions: usize,
    pub newest: Option<SystemTime>,
}

impl Project {
    pub fn memory_dir(&self) -> PathBuf {
        self.dir.join("memory")
    }
}

pub fn scan() -> Vec<Project> {
    let root = claude_dir().join("projects");
    let mut out = Vec::new();
    for e in std::fs::read_dir(&root).into_iter().flatten().filter_map(Result::ok) {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        let mut sessions = Vec::new();
        let mut newest = mtime(&dir);
        for f in std::fs::read_dir(&dir).into_iter().flatten().filter_map(Result::ok) {
            let p = f.path();
            if p.extension().is_some_and(|x| x == "jsonl") {
                let t = mtime(&p);
                if t > newest {
                    newest = t;
                }
                sessions.push(p);
            }
        }
        let (mut mem_files, mut mem_bytes) = (0, 0);
        for f in std::fs::read_dir(dir.join("memory")).into_iter().flatten().filter_map(Result::ok) {
            if let Ok(m) = f.metadata() {
                if m.is_file() {
                    mem_files += 1;
                    mem_bytes += m.len();
                    let t = m.modified().ok();
                    if t > newest {
                        newest = t;
                    }
                }
            }
        }
        let real = sessions
            .iter()
            .find_map(|s| cwd_from_session(s))
            .or_else(|| decode(&name));
        let exists = real.as_ref().is_some_and(|p| p.exists());
        out.push(Project {
            dir,
            name,
            real,
            exists,
            mem_files,
            mem_bytes,
            sessions: sessions.len(),
            newest,
        });
    }
    out.sort_by(|a, b| {
        a.exists
            .cmp(&b.exists)
            .then(b.mem_bytes.cmp(&a.mem_bytes))
            .then(a.name.cmp(&b.name))
    });
    out
}

/// Read the `"cwd"` field from the start of a session transcript.
fn cwd_from_session(p: &Path) -> Option<PathBuf> {
    let mut buf = vec![0u8; 256 * 1024];
    let n = std::fs::File::open(p).ok()?.read(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf[..n]);
    let i = text.find("\"cwd\":\"")? + 7;
    let rest = &text[i..];
    let end = rest.find('"')?;
    Some(PathBuf::from(rest[..end].replace("\\\\", "\\")))
}

/// Find an existing path whose encoded form equals `enc`. The encoding is lossy,
/// so the search walks the filesystem and matches each directory level.
fn decode(enc: &str) -> Option<PathBuf> {
    fn rec(dir: &Path, rest: &str, budget: &mut usize) -> Option<PathBuf> {
        if rest.is_empty() {
            return Some(dir.to_path_buf());
        }
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        let mut entries: Vec<_> = std::fs::read_dir(dir).ok()?.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.file_name().len()));
        for e in entries {
            let n = encode_str(&e.file_name().to_string_lossy());
            if rest == n {
                return Some(e.path());
            }
            if let Some(r) = rest.strip_prefix(&n).and_then(|r| r.strip_prefix('-')) {
                if e.path().is_dir() {
                    if let Some(p) = rec(&e.path(), r, budget) {
                        return Some(p);
                    }
                }
            }
        }
        None
    }
    let rest = enc.strip_prefix('-')?;
    let mut budget = 2000;
    rec(Path::new("/"), rest, &mut budget)
}
