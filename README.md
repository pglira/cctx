# cctx

A terminal UI that shows what Claude Code loads into its context for a directory,
and helps you remove outdated content.

```
cctx [DIR]              # TUI for DIR (default: current directory)
cctx --list [DIR]       # same data as text
cctx --projects         # all per-project data directories, with "gone" status
cctx --stale-days N     # age threshold for the "old" flag (default 90)
```

## What it shows

For a session started in `DIR`:

| Group | Sources | Load |
|---|---|---|
| Instructions | `/etc/claude-code/CLAUDE.md`, `~/.claude/CLAUDE.md`, `~/.claude/rules/`, `CLAUDE.md` / `.claude/CLAUDE.md` / `CLAUDE.local.md` in `DIR` and every parent, `@imports`, `.claude/rules/` | ● always |
| | `CLAUDE.md` in subdirectories (git-tracked or not ignored) | ○ on demand |
| Memory | `~/.claude/projects/<encoded git root or DIR>/memory/MEMORY.md` (first 200 lines) | ● always |
| | other memory files | ○ on demand |
| Skills | `~/.claude/skills`, `.claude/skills` in `DIR` and parents, enabled plugins | ◐ description only |
| Commands, Agents | user, project, and plugin directories | ◐ description only |
| Output styles | user and project `output-styles/` | ● if active |
| Settings | user and project `settings*.json`, `.mcp.json` | · not loaded as text |

The header shows an estimate of the tokens that these files add at session start
(four characters per token). The system prompt, tool definitions, and MCP
instructions are not counted.

## Flags

- A path (absolute or `~/`) in the text does not exist.
- An `@import` does not exist.
- A `MEMORY.md` line links to a missing file, or a memory file is not in the index.
- `MEMORY.md` has more than 200 lines (the rest does not load).
- An instruction or memory file has not changed for `--stale-days` days.

## Keys

| Key | Action |
|---|---|
| `j`/`k`, `g`/`G` | move |
| `J`/`K`, `Ctrl-d`/`Ctrl-u` | scroll the preview |
| `e`, `Enter` | open in `$VISUAL` / `$EDITOR` |
| `d` | move to the trash; for a memory file, also remove its line from `MEMORY.md` |
| `r` | review with `claude -p` (tools: Read, Glob, Grep only), then show a diff; `a` accepts, `n` rejects |
| `s` | turn a skill off or on through `skillOverrides` in the user settings |
| `f` | show flagged items only |
| `p` | all-projects view: `Enter` opens a project, `d` trashes its memory, `D` trashes all its data |
| `c` | return to the start directory |
| `R` | rescan |

When you accept a review, the previous version of the file goes to the trash.
`$CLAUDE_CONFIG_DIR` overrides `~/.claude`.

## Build

```
cargo install --path .
```
