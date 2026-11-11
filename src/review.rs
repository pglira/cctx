use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

pub struct Review {
    pub path: PathBuf,
    pub original: String,
    pub revised: String,
    pub notes: String,
}

fn prompt(path: &Path, role: &str, cwd: &Path, content: &str) -> String {
    format!(
        "You review one file that Claude Code loads into its context.\n\
The goal: remove content that is outdated, wrong, duplicated, or of low value, \
so that the context stays small and correct.\n\n\
File: {path}\n\
Role of the file: {role}\n\
Working directory of the user: {cwd}\n\n\
Rules:\n\
- Check facts against the filesystem with Read, Glob, and Grep where you can \
(paths, file names, commands, settings, project structure).\n\
- Remove content that is outdated, that contradicts the current state, that duplicates \
other loaded instructions, or that is too specific to justify its context cost.\n\
- Keep all valid content verbatim. Do not rewrite style or wording of valid content.\n\
- Keep the frontmatter and the file format.\n\
- Do not modify any files.\n\n\
Output exactly two blocks and nothing else:\n\
<notes>\n- one bullet per removal or change, with the reason\n</notes>\n\
<revised>\nthe full new file content\n</revised>\n\
If nothing must change, put the original content unchanged in <revised> and say so in <notes>.\n\n\
File content:\n<file>\n{content}\n</file>\n",
        path = path.display(),
        cwd = cwd.display(),
    )
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let i = s.find(open)? + open.len();
    let j = s[i..].rfind(close)? + i;
    Some(&s[i..j])
}

/// Run `claude -p` with read-only tools in a background thread and send the result.
pub fn spawn(path: PathBuf, role: String, cwd: PathBuf, content: String, tx: Sender<Result<Review, String>>) {
    std::thread::spawn(move || {
        let res = run(&path, &role, &cwd, &content);
        let _ = tx.send(res);
    });
}

fn run(path: &Path, role: &str, cwd: &Path, content: &str) -> Result<Review, String> {
    let dir = if cwd.is_dir() { cwd.to_path_buf() } else { crate::util::home() };
    let mut child = Command::new("claude")
        .args([
            "-p",
            "--tools",
            "Read,Glob,Grep",
            "--no-session-persistence",
            "--output-format",
            "text",
        ])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start claude: {e}"))?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(prompt(path, role, cwd, content).as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("claude failed: {} {}", err.trim(), stdout.trim()));
    }
    let notes = between(&stdout, "<notes>", "</notes>").unwrap_or("").trim().to_string();
    let Some(revised) = between(&stdout, "<revised>", "</revised>") else {
        return Err(format!("unexpected output from claude: {}", crate::util::truncate(stdout.trim(), 300)));
    };
    let mut revised = revised.strip_prefix('\n').unwrap_or(revised).to_string();
    while revised.ends_with("\n\n") {
        revised.pop();
    }
    if content.ends_with('\n') && !revised.ends_with('\n') {
        revised.push('\n');
    }
    if !content.ends_with('\n') && revised.ends_with('\n') {
        revised.pop();
    }
    Ok(Review {
        path: path.to_path_buf(),
        original: content.to_string(),
        revised,
        notes,
    })
}
