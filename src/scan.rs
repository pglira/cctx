use crate::model::*;
use crate::stale;
use crate::util::*;
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Lines of MEMORY.md that load at session start.
pub const INDEX_LINES: usize = 200;
const MAX_IMPORT_DEPTH: usize = 5;
const SUBDIR_DEPTH: usize = 4;
const SUBDIR_ENTRY_LIMIT: usize = 20_000;

#[derive(Default, Clone)]
pub struct Settings {
    pub output_style: Option<String>,
    pub skill_overrides: HashMap<String, String>,
    pub enabled_plugins: HashSet<String>,
}

#[derive(Clone)]
pub struct Ctx {
    pub cwd: PathBuf,
    pub home: PathBuf,
    pub claude_dir: PathBuf,
    pub project_root: PathBuf,
    pub memory_dir: PathBuf,
    pub stale_days: u64,
    pub settings: Settings,
}

pub fn git_root(p: &Path) -> Option<PathBuf> {
    p.ancestors()
        .find(|a| a.join(".git").exists())
        .map(Path::to_path_buf)
}

impl Ctx {
    pub fn new(cwd: PathBuf, memory_override: Option<PathBuf>, stale_days: u64) -> Ctx {
        let home = home();
        let claude_dir = claude_dir();
        let project_root = git_root(&cwd).unwrap_or_else(|| cwd.clone());
        let memory_dir = memory_override.unwrap_or_else(|| {
            claude_dir
                .join("projects")
                .join(encode(&project_root))
                .join("memory")
        });
        let mut ctx = Ctx {
            cwd,
            home,
            claude_dir,
            project_root,
            memory_dir,
            stale_days,
            settings: Settings::default(),
        };
        ctx.settings = ctx.load_settings();
        ctx
    }

    pub fn user_settings(&self) -> PathBuf {
        self.claude_dir.join("settings.json")
    }

    /// Settings files in order of increasing precedence.
    pub fn settings_files(&self) -> Vec<PathBuf> {
        let mut v = vec![
            self.claude_dir.join("settings.json"),
            self.claude_dir.join("settings.local.json"),
        ];
        let mut roots = vec![self.project_root.clone()];
        if self.cwd != self.project_root {
            roots.push(self.cwd.clone());
        }
        for r in roots {
            if r == self.home {
                continue;
            }
            v.push(r.join(".claude/settings.json"));
            v.push(r.join(".claude/settings.local.json"));
        }
        v
    }

    fn load_settings(&self) -> Settings {
        let mut s = Settings::default();
        for f in self.settings_files() {
            let Some(text) = read(&f) else { continue };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(o) = v.get("outputStyle").and_then(Value::as_str) {
                s.output_style = Some(o.to_string());
            }
            if let Some(m) = v.get("skillOverrides").and_then(Value::as_object) {
                for (k, val) in m {
                    if let Some(val) = val.as_str() {
                        s.skill_overrides.insert(k.clone(), val.to_string());
                    }
                }
            }
            if let Some(m) = v.get("enabledPlugins").and_then(Value::as_object) {
                for (k, val) in m {
                    if val.as_bool() == Some(true) {
                        s.enabled_plugins.insert(k.clone());
                    } else {
                        s.enabled_plugins.remove(k);
                    }
                }
            }
        }
        s
    }

    /// Directories from the filesystem root down to `cwd`.
    fn ancestors(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.cwd.ancestors().map(Path::to_path_buf).collect();
        v.reverse();
        v
    }

    fn origin_of(&self, dir: &Path) -> String {
        if dir == self.cwd {
            "project".into()
        } else {
            "parent".into()
        }
    }

    /// Project-level `.claude` directories: `cwd` and its parents, without the home directory.
    fn project_claude_dirs(&self) -> Vec<(PathBuf, String)> {
        self.ancestors()
            .into_iter()
            .filter(|d| *d != self.home)
            .map(|d| (d.join(".claude"), self.origin_of(&d)))
            .filter(|(d, _)| d.is_dir())
            .collect()
    }
}

fn link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\]\(([^)\s]+)\)").unwrap())
}

fn import_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|\s)@([~./A-Za-z0-9_][^\s]*)").unwrap())
}

fn inline_code_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"`[^`]*`").unwrap())
}

/// `@path` imports outside code, as (line number, path).
fn imports(content: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut fenced = false;
    for (i, line) in content.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let clean = inline_code_re().replace_all(line, "");
        for cap in import_re().captures_iter(&clean) {
            let p = cap[1].trim_end_matches(['.', ',', ';', ':', ')']);
            if p.contains('/') || p.ends_with(".md") {
                out.push((i + 1, p.to_string()));
            }
        }
    }
    out
}

struct Scanner<'a> {
    ctx: &'a Ctx,
    items: Vec<Item>,
    seen: HashSet<PathBuf>,
}

pub fn scan(ctx: &Ctx) -> Vec<Item> {
    let mut s = Scanner {
        ctx,
        items: Vec::new(),
        seen: HashSet::new(),
    };
    s.instructions();
    s.memory();
    s.skills();
    s.commands_agents_styles();
    s.config();
    for it in &mut s.items {
        stale::annotate(it, ctx.stale_days);
    }
    s.items
}

impl Scanner<'_> {
    /// Path relative to `cwd` when below it, else with `~` for the home directory.
    fn show(&self, p: &Path) -> String {
        match p.strip_prefix(&self.ctx.cwd) {
            Ok(rel) if self.ctx.cwd != self.ctx.home => format!("./{}", rel.display()),
            _ => tilde(p),
        }
    }

    fn first_visit(&mut self, p: &Path) -> bool {
        let c = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        self.seen.insert(c)
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        group: Group,
        kind: Kind,
        path: &Path,
        label: String,
        origin: &str,
        desc: Option<String>,
        load: Load,
        tokens_loaded: usize,
        content: String,
        read_only: bool,
        depth: usize,
    ) -> usize {
        self.items.push(Item {
            group,
            kind,
            path: path.to_path_buf(),
            label,
            origin: origin.to_string(),
            desc,
            load,
            tokens_loaded: if load.counts() { tokens_loaded } else { 0 },
            tokens_full: tokens(&content),
            mtime: mtime(path),
            flags: Vec::new(),
            read_only,
            content,
            depth,
        });
        self.items.len() - 1
    }

    // ---- instructions ----

    fn instructions(&mut self) {
        let ctx = self.ctx;
        self.md(Path::new("/etc/claude-code/CLAUDE.md"), Kind::ClaudeMd, "managed", Load::Always, 0);
        self.md(&ctx.claude_dir.join("CLAUDE.md"), Kind::ClaudeMd, "user", Load::Always, 0);
        self.rules(&ctx.claude_dir.join("rules"), "user");
        for dir in ctx.ancestors() {
            let origin = ctx.origin_of(&dir);
            for f in ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"] {
                self.md(&dir.join(f), Kind::ClaudeMd, &origin, Load::Always, 0);
            }
            if dir != ctx.home {
                self.rules(&dir.join(".claude/rules"), &origin);
            }
        }
        self.subdir_claude_md();
    }

    fn md(&mut self, path: &Path, kind: Kind, origin: &str, load: Load, depth: usize) {
        if !path.is_file() || !self.first_visit(path) {
            return;
        }
        let content = read(path).unwrap_or_default();
        let t = tokens(&content);
        let idx = self.push(
            Group::Instructions,
            kind,
            path,
            self.show(path),
            origin,
            None,
            load,
            t,
            content.clone(),
            false,
            depth,
        );
        if depth >= MAX_IMPORT_DEPTH {
            return;
        }
        // Resolve relative imports against the file's directory; for a file inside
        // a `.claude` directory, also against the directory that contains `.claude`.
        let base = path.parent().unwrap_or(Path::new("/")).to_path_buf();
        let mut bases = vec![base.clone()];
        if base.file_name().is_some_and(|n| n == ".claude") {
            if let Some(up) = base.parent() {
                bases.push(up.to_path_buf());
            }
        }
        for (line, imp) in imports(&content) {
            let p = bases
                .iter()
                .map(|b| expand(&imp, b))
                .find(|p| p.exists())
                .unwrap_or_else(|| expand(&imp, &base));
            if p.is_file() {
                self.md(&p, Kind::Import, origin, load, depth + 1);
            } else if !p.exists() {
                self.items[idx].flags.push(Flag::MissingImport { line, path: imp });
            }
        }
    }

    fn rules(&mut self, dir: &Path, origin: &str) {
        if !dir.is_dir() {
            return;
        }
        let mut files: Vec<PathBuf> = walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "md"))
            .map(|e| e.into_path())
            .collect();
        files.sort();
        for f in files {
            if !self.first_visit(&f) {
                continue;
            }
            let content = read(&f).unwrap_or_default();
            let fm = frontmatter(&content);
            let paths = fm_get(&fm, "paths");
            let load = if paths.is_some() { Load::OnDemand } else { Load::Always };
            let t = tokens(&content);
            self.push(
                Group::Instructions,
                Kind::Rule,
                &f,
                self.show(&f),
                origin,
                paths.map(|p| format!("applies to paths: {p}")),
                load,
                t,
                content,
                false,
                0,
            );
        }
    }

    /// CLAUDE.md files below `cwd`. They load when Claude reads files in their directory.
    fn subdir_claude_md(&mut self) {
        let mut found = git_claude_md(&self.ctx.cwd).unwrap_or_else(|| walk_claude_md(&self.ctx.cwd));
        found.sort();
        for f in found {
            self.md(&f, Kind::ClaudeMd, "subdir", Load::OnDemand, 0);
        }
    }

    // ---- memory ----

    fn memory(&mut self) {
        let dir = self.ctx.memory_dir.clone();
        if !dir.is_dir() {
            return;
        }
        let index = dir.join("MEMORY.md");
        let mut linked = HashSet::new();
        if index.is_file() {
            let content = read(&index).unwrap_or_default();
            let lines: Vec<&str> = content.lines().collect();
            let loaded = lines.iter().take(INDEX_LINES).cloned().collect::<Vec<_>>().join("\n");
            let mut flags = Vec::new();
            for (i, line) in lines.iter().enumerate() {
                for cap in link_re().captures_iter(line) {
                    let target = cap[1].split('#').next().unwrap_or("").to_string();
                    if target.is_empty() || target.contains("://") {
                        continue;
                    }
                    linked.insert(target.clone());
                    if !dir.join(&target).exists() {
                        flags.push(Flag::BrokenIndexLink { line: i + 1, target });
                    }
                }
            }
            if lines.len() > INDEX_LINES {
                flags.push(Flag::IndexTruncated { lines: lines.len() });
            }
            self.first_visit(&index);
            let idx = self.push(
                Group::Memory,
                Kind::MemoryIndex,
                &index,
                "MEMORY.md".into(),
                "memory",
                None,
                Load::Always,
                tokens(&loaded),
                content,
                false,
                0,
            );
            self.items[idx].flags = flags;
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "md"))
            .filter(|p| p.file_name().is_some_and(|n| n != "MEMORY.md"))
            .collect();
        files.sort();
        for f in files {
            let name = f.file_name().unwrap().to_string_lossy().to_string();
            let content = read(&f).unwrap_or_default();
            let fm = frontmatter(&content);
            let desc = fm_get(&fm, "description");
            let idx = self.push(
                Group::Memory,
                Kind::MemoryFile,
                &f,
                name.clone(),
                "memory",
                desc,
                Load::OnDemand,
                0,
                content,
                false,
                1,
            );
            if index.is_file() && !linked.contains(&name) {
                self.items[idx].flags.push(Flag::Unindexed);
            }
        }
    }

    // ---- skills ----

    fn skills(&mut self) {
        let ctx = self.ctx;
        self.skill_dir(&ctx.claude_dir.join("skills"), "user", false, None);
        for (d, origin) in ctx.project_claude_dirs() {
            self.skill_dir(&d.join("skills"), &origin, false, None);
        }
        for (name, path) in self.plugins() {
            let origin = format!("plugin:{name}");
            self.skill_dir(&path.join("skills"), &origin, true, Some(&name));
        }
    }

    /// Enabled plugins as (short name, install path).
    fn plugins(&self) -> Vec<(String, PathBuf)> {
        let f = self.ctx.claude_dir.join("plugins/installed_plugins.json");
        let Some(text) = read(&f) else { return vec![] };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            return vec![];
        };
        let mut out = Vec::new();
        if let Some(m) = v.get("plugins").and_then(Value::as_object) {
            for (key, entries) in m {
                if !self.ctx.settings.enabled_plugins.contains(key) {
                    continue;
                }
                let name = key.split('@').next().unwrap_or(key).to_string();
                for e in entries.as_array().into_iter().flatten() {
                    if let Some(p) = e.get("installPath").and_then(Value::as_str) {
                        out.push((name.clone(), PathBuf::from(p)));
                        break;
                    }
                }
            }
        }
        out.sort();
        out
    }

    fn skill_dir(&mut self, dir: &Path, origin: &str, read_only: bool, prefix: Option<&str>) {
        let mut subs: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path().join("SKILL.md"))
            .filter(|p| p.is_file())
            .collect();
        subs.sort();
        for f in subs {
            if !self.first_visit(&f) {
                continue;
            }
            let content = read(&f).unwrap_or_default();
            let fm = frontmatter(&content);
            let dirname = f.parent().and_then(Path::file_name).unwrap().to_string_lossy().to_string();
            let name = fm_get(&fm, "name").unwrap_or(dirname);
            let desc = fm_get(&fm, "description").unwrap_or_default();
            let key = match prefix {
                Some(p) => format!("{p}:{name}"),
                None => name.clone(),
            };
            let manual_only = fm_get(&fm, "disable-model-invocation").as_deref() == Some("true");
            let load = if self.ctx.settings.skill_overrides.get(&key).map(String::as_str) == Some("off") {
                Load::Disabled
            } else if manual_only {
                Load::OnDemand
            } else {
                Load::Description
            };
            let t = tokens(&name) + tokens(&desc) + 4;
            self.push(
                Group::Skills,
                Kind::Skill { key: key.clone() },
                &f,
                key,
                origin,
                Some(desc),
                load,
                t,
                content,
                read_only,
                0,
            );
        }
    }

    // ---- commands, agents, output styles ----

    fn commands_agents_styles(&mut self) {
        let ctx = self.ctx;
        let mut dirs = vec![(ctx.claude_dir.clone(), "user".to_string(), false)];
        for (d, o) in ctx.project_claude_dirs() {
            dirs.push((d, o, false));
        }
        for (name, p) in self.plugins() {
            dirs.push((p, format!("plugin:{name}"), true));
        }
        for (d, origin, ro) in &dirs {
            self.md_defs(&d.join("commands"), Group::Commands, Kind::Command, origin, *ro, true);
        }
        for (d, origin, ro) in &dirs {
            self.md_defs(&d.join("agents"), Group::Agents, Kind::Agent, origin, *ro, true);
        }
        for (d, origin, ro) in &dirs {
            self.md_defs(&d.join("output-styles"), Group::OutputStyles, Kind::OutputStyle, origin, *ro, false);
        }
    }

    fn md_defs(&mut self, dir: &Path, group: Group, kind: Kind, origin: &str, read_only: bool, recurse: bool) {
        if !dir.is_dir() {
            return;
        }
        let mut files: Vec<PathBuf> = walkdir::WalkDir::new(dir)
            .max_depth(if recurse { 8 } else { 1 })
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "md"))
            .map(|e| e.into_path())
            .collect();
        files.sort();
        for f in files {
            if !self.first_visit(&f) {
                continue;
            }
            let content = read(&f).unwrap_or_default();
            let fm = frontmatter(&content);
            let rel = f.strip_prefix(dir).unwrap_or(&f).with_extension("");
            let stem = rel.to_string_lossy().replace('/', ":");
            let name = fm_get(&fm, "name").unwrap_or_else(|| stem.clone());
            let desc = fm_get(&fm, "description").or_else(|| {
                body(&content)
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .map(|l| truncate(l, 120))
            });
            let (label, load) = match kind {
                Kind::Command => (format!("/{stem}"), Load::Description),
                Kind::OutputStyle => {
                    let active = self
                        .ctx
                        .settings
                        .output_style
                        .as_ref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(&name) || s.eq_ignore_ascii_case(&stem));
                    (name.clone(), if active { Load::Always } else { Load::Never })
                }
                _ => (name.clone(), Load::Description),
            };
            let t = match load {
                Load::Always => tokens(body(&content)),
                _ => tokens(&label) + tokens(desc.as_deref().unwrap_or("")) + 4,
            };
            self.push(group, kind.clone(), &f, label, origin, desc, load, t, content, read_only, 0);
        }
    }

    // ---- settings ----

    fn config(&mut self) {
        let ctx = self.ctx;
        let mut files = ctx.settings_files();
        files.push(ctx.project_root.join(".mcp.json"));
        if ctx.cwd != ctx.project_root {
            files.push(ctx.cwd.join(".mcp.json"));
        }
        for f in files {
            if !f.is_file() || !self.first_visit(&f) {
                continue;
            }
            let content = read(&f).unwrap_or_default();
            let origin = if f.starts_with(&ctx.claude_dir) { "user" } else { "project" };
            self.push(
                Group::Config,
                Kind::Settings,
                &f,
                tilde(&f),
                origin,
                None,
                Load::Never,
                0,
                content,
                false,
                0,
            );
        }
    }
}

/// CLAUDE.md files below `cwd` that git does not ignore. `None` outside a git repo.
fn git_claude_md(cwd: &Path) -> Option<Vec<PathBuf>> {
    git_root(cwd)?;
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["ls-files", "-co", "--exclude-standard", "--", "*CLAUDE.md", "*CLAUDE.local.md"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.contains('/'))
            .filter(|l| l.ends_with("/CLAUDE.md") || l.ends_with("/CLAUDE.local.md"))
            .map(|l| cwd.join(l))
            .collect(),
    )
}

/// CLAUDE.md files below `cwd`, found by a bounded directory walk.
fn walk_claude_md(cwd: &Path) -> Vec<PathBuf> {
    const SKIP: &[&str] = &[
        "node_modules", "target", "venv", "__pycache__", "build", "dist", "vendor", "_deps",
    ];
    let mut found = Vec::new();
        let walker = walkdir::WalkDir::new(cwd)
            .max_depth(SUBDIR_DEPTH)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 || !e.file_type().is_dir() {
                    return true;
                }
                let n = e.file_name().to_string_lossy();
                !n.starts_with('.') && !SKIP.contains(&n.as_ref())
            });
        for e in walker.take(SUBDIR_ENTRY_LIMIT).filter_map(Result::ok) {
            if e.depth() >= 2 && e.file_type().is_file() {
                let n = e.file_name().to_string_lossy();
                if n == "CLAUDE.md" || n == "CLAUDE.local.md" {
                    found.push(e.into_path());
                }
            }
        }
        found
}

