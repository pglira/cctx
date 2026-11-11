use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub enum Group {
    Instructions,
    Memory,
    Skills,
    Commands,
    Agents,
    OutputStyles,
    Config,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Instructions => "Instructions",
            Group::Memory => "Memory",
            Group::Skills => "Skills",
            Group::Commands => "Commands",
            Group::Agents => "Agents",
            Group::OutputStyles => "Output styles",
            Group::Config => "Settings",
        }
    }
}

/// How an item enters the context of a session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Load {
    /// Full text loads at session start.
    Always,
    /// Only name and description load; the body loads on use.
    Description,
    /// Loads only when Claude reads it or a matching file.
    OnDemand,
    /// Turned off in `skillOverrides`.
    Disabled,
    /// Not loaded as text (settings, inactive output styles).
    Never,
}

impl Load {
    pub fn marker(self) -> &'static str {
        match self {
            Load::Always => "●",
            Load::Description => "◐",
            Load::OnDemand => "○",
            Load::Disabled => "✗",
            Load::Never => "·",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Load::Always => "always (full text, every session)",
            Load::Description => "description only (body loads on use)",
            Load::OnDemand => "on demand",
            Load::Disabled => "disabled (skillOverrides)",
            Load::Never => "not loaded as text",
        }
    }

    pub fn counts(self) -> bool {
        matches!(self, Load::Always | Load::Description)
    }
}

#[derive(Clone, Debug)]
pub enum Flag {
    MissingPath { line: usize, path: String },
    MissingImport { line: usize, path: String },
    BrokenIndexLink { line: usize, target: String },
    Unindexed,
    IndexTruncated { lines: usize },
    Old { days: u64 },
}

impl Flag {
    pub fn line(&self) -> Option<usize> {
        match self {
            Flag::MissingPath { line, .. }
            | Flag::MissingImport { line, .. }
            | Flag::BrokenIndexLink { line, .. } => Some(*line),
            _ => None,
        }
    }

    pub fn text(&self) -> String {
        match self {
            Flag::MissingPath { line, path } => format!("line {line}: path does not exist: {path}"),
            Flag::MissingImport { line, path } => format!("line {line}: import does not exist: @{path}"),
            Flag::BrokenIndexLink { line, target } => {
                format!("line {line}: index points to missing file: {target}")
            }
            Flag::Unindexed => "memory file is not listed in MEMORY.md".into(),
            Flag::IndexTruncated { lines } => {
                format!("index has {lines} lines; only the first 200 load")
            }
            Flag::Old { days } => format!("not changed for {days} days"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    ClaudeMd,
    Import,
    Rule,
    MemoryIndex,
    MemoryFile,
    Skill { key: String },
    Command,
    Agent,
    OutputStyle,
    Settings,
}

impl Kind {
    pub fn role(&self) -> &'static str {
        match self {
            Kind::ClaudeMd => "a CLAUDE.md instruction file that loads into every session",
            Kind::Import => "a file imported with @ from a CLAUDE.md; it loads into every session",
            Kind::Rule => "a rule file from a .claude/rules directory",
            Kind::MemoryIndex => {
                "the MEMORY.md index of the auto memory; it loads into every session"
            }
            Kind::MemoryFile => "one auto-memory file; it loads when Claude recalls it",
            Kind::Skill { .. } => {
                "a skill (SKILL.md); its description loads every session, its body on use"
            }
            Kind::Command => "a custom slash command",
            Kind::Agent => "a subagent definition",
            Kind::OutputStyle => "an output style",
            Kind::Settings => "a Claude Code settings file (JSON)",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub group: Group,
    pub kind: Kind,
    pub path: PathBuf,
    pub label: String,
    pub origin: String,
    pub desc: Option<String>,
    pub load: Load,
    pub tokens_loaded: usize,
    pub tokens_full: usize,
    pub mtime: Option<SystemTime>,
    pub flags: Vec<Flag>,
    pub read_only: bool,
    pub content: String,
    pub depth: usize,
}
