use crate::app::{show_project, App, Modal, Row, View};
use crate::model::*;
use crate::util::*;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use similar::{ChangeTag, TextDiff};

const DIM: Color = Color::DarkGray;
const WARN: Color = Color::Yellow;
const BAD: Color = Color::Red;

pub fn draw(f: &mut Frame, app: &mut App) {
    let [top, mid, bot] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
    header(f, top, app);
    match app.view {
        View::Main => main_view(f, mid, app),
        View::Projects => projects_view(f, mid, app),
    }
    footer(f, bot, app);
    modal(f, app);
}

fn header(f: &mut Frame, area: Rect, app: &App) {
    let ctx = &app.ctx;
    let mut l1 = vec![
        Span::styled(" cctx ", Style::new().bg(Color::Magenta).fg(Color::Black).bold()),
        Span::raw("  "),
    ];
    if let Some(p) = &app.viewing_project {
        l1.push(Span::styled("project: ", Style::new().fg(DIM)));
        l1.push(Span::styled(p.clone(), Style::new().fg(Color::Cyan)));
    } else {
        l1.push(Span::styled("dir: ", Style::new().fg(DIM)));
        l1.push(Span::raw(tilde(&ctx.cwd)));
    }
    if ctx.project_root != ctx.cwd {
        l1.push(Span::styled("  root: ", Style::new().fg(DIM)));
        l1.push(Span::raw(tilde(&ctx.project_root)));
    }
    let mem = if ctx.memory_dir.is_dir() { tilde(&ctx.memory_dir) } else { "none".into() };
    let l2 = vec![
        Span::styled(" loaded at start ≈ ", Style::new().fg(DIM)),
        Span::styled(format!("{} tok", fmt_tokens(app.total_tokens())), Style::new().bold()),
        Span::styled(" (files only; system prompt, tools, MCP not counted)", Style::new().fg(DIM)),
        Span::styled("   flagged: ", Style::new().fg(DIM)),
        Span::styled(
            app.flagged_count().to_string(),
            Style::new().fg(if app.flagged_count() > 0 { WARN } else { Color::Green }),
        ),
        Span::styled("   memory: ", Style::new().fg(DIM)),
        Span::raw(mem),
    ];
    f.render_widget(Paragraph::new(vec![Line::from(l1), Line::from(l2)]), area);
}

fn footer(f: &mut Frame, area: Rect, app: &App) {
    let keys = match app.view {
        View::Main => "j/k move  J/K scroll  e edit  d trash  r review  s skill on/off  f flagged  p projects  ? help  q quit",
        View::Projects => "j/k move  enter open  e edit index  d trash memory  D trash all  f gone only  esc back  q quit",
    };
    let status = if let Some((name, t)) = &app.reviewing {
        let spin = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let s = t.elapsed().as_millis() / 120;
        format!("{} claude reviews {} … {}s", spin[s as usize % spin.len()], name, t.elapsed().as_secs())
    } else {
        app.status.clone()
    };
    let w = area.width as usize;
    let sw = status.chars().count().min(w / 2);
    let kw = w.saturating_sub(sw + 2);
    let line = Line::from(vec![
        Span::styled(truncate(keys, kw), Style::new().fg(DIM)),
        Span::raw(" ".repeat(kw.saturating_sub(keys.chars().count().min(kw)) + 2)),
        Span::styled(truncate(&status, sw), Style::new().fg(Color::Cyan)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn load_style(l: Load) -> Style {
    match l {
        Load::Always => Style::new().fg(Color::Green),
        Load::Description => Style::new().fg(Color::Cyan),
        Load::OnDemand => Style::new().fg(Color::Blue),
        Load::Disabled => Style::new().fg(DIM),
        Load::Never => Style::new().fg(DIM),
    }
}

fn main_view(f: &mut Frame, area: Rect, app: &mut App) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    let inner_w = left.width.saturating_sub(4) as usize;
    let rows: Vec<ListItem> = app
        .rows
        .iter()
        .map(|r| match r {
            Row::Header(g) => {
                let t = app.group_tokens(*g);
                let mut spans = vec![Span::styled(
                    g.title().to_uppercase(),
                    Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
                )];
                if t > 0 {
                    spans.push(Span::styled(format!("  ≈{} tok", fmt_tokens(t)), Style::new().fg(DIM)));
                }
                ListItem::new(Line::from(spans))
            }
            Row::Item(i) => item_row(&app.items[*i], inner_w),
        })
        .collect();
    let title = if app.only_flagged { " Context (flagged only) " } else { " Context " };
    let list = List::new(rows)
        .block(Block::new().borders(Borders::ALL).title(title))
        .highlight_style(Style::new().bg(Color::Rgb(50, 50, 70)).add_modifier(Modifier::BOLD))
        .highlight_symbol("▌");
    f.render_stateful_widget(list, left, &mut app.list);

    let block = Block::new().borders(Borders::ALL);
    match app.selected() {
        Some(it) => {
            let text = item_preview(it);
            let p = Paragraph::new(text)
                .block(block.title(format!(" {} ", it.label)))
                .wrap(Wrap { trim: false })
                .scroll((app.preview_scroll, 0));
            f.render_widget(p, right);
        }
        None => f.render_widget(Paragraph::new("nothing found").block(block), right),
    }
}

fn item_row(it: &Item, w: usize) -> ListItem<'static> {
    let tok = match it.load {
        Load::Always | Load::Description => fmt_tokens(it.tokens_loaded),
        _ => format!("({})", fmt_tokens(it.tokens_full)),
    };
    let age = fmt_age(age_days(it.mtime));
    let flags = if it.flags.is_empty() { String::new() } else { format!("!{}", it.flags.len()) };
    let indent = if it.depth > 0 { "  ".repeat(it.depth.min(3)) + "↳ " } else { String::new() };
    let origin = if matches!(it.group, Group::Skills | Group::Commands | Group::Agents | Group::OutputStyles | Group::Instructions)
        && it.origin != "user"
    {
        format!(" [{}]", it.origin)
    } else {
        String::new()
    };
    let fixed = 2 + 7 + 5 + 4;
    let lw = w.saturating_sub(fixed);
    let label = truncate(&format!("{indent}{}{origin}", it.label), lw);
    let pad = lw.saturating_sub(label.chars().count());
    let name_style = if it.load == Load::Disabled {
        Style::new().fg(DIM).add_modifier(Modifier::CROSSED_OUT)
    } else {
        Style::new()
    };
    let has_bad = it.flags.iter().any(|f| !matches!(f, Flag::Old { .. }));
    ListItem::new(Line::from(vec![
        Span::styled(format!("{} ", it.load.marker()), load_style(it.load)),
        Span::styled(label, name_style),
        Span::raw(" ".repeat(pad)),
        Span::styled(format!("{tok:>7}"), load_style(it.load)),
        Span::styled(format!("{age:>5}"), Style::new().fg(DIM)),
        Span::styled(format!("{flags:>4}"), Style::new().fg(if has_bad { BAD } else { WARN })),
    ]))
}

fn kv(k: &str, v: String) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{k:<10}"), Style::new().fg(DIM)), Span::raw(v)])
}

fn item_preview(it: &Item) -> Text<'static> {
    let mut lines = vec![
        kv("path", tilde(&it.path)),
        kv("origin", if it.read_only { format!("{} (read-only)", it.origin) } else { it.origin.clone() }),
        Line::from(vec![
            Span::styled(format!("{:<10}", "loads"), Style::new().fg(DIM)),
            Span::styled(it.load.describe().to_string(), load_style(it.load)),
        ]),
        kv(
            "tokens",
            format!("≈{} at start, ≈{} full text", fmt_tokens(it.tokens_loaded), fmt_tokens(it.tokens_full)),
        ),
        kv("changed", format!("{} ago", fmt_age(age_days(it.mtime)))),
    ];
    if let Some(d) = &it.desc {
        if !d.is_empty() {
            lines.push(kv("about", d.clone()));
        }
    }
    for fl in &it.flags {
        let c = if matches!(fl, Flag::Old { .. }) { WARN } else { BAD };
        lines.push(Line::from(Span::styled(format!("! {}", fl.text()), Style::new().fg(c))));
    }
    lines.push(Line::from(Span::styled("─".repeat(60), Style::new().fg(DIM))));
    let bad_lines: Vec<usize> = it.flags.iter().filter_map(Flag::line).collect();
    let loaded_limit = if it.kind == Kind::MemoryIndex { Some(crate::scan::INDEX_LINES) } else { None };
    for (i, l) in it.content.lines().enumerate() {
        let n = i + 1;
        let bad = bad_lines.contains(&n);
        let cut = loaded_limit.is_some_and(|m| n > m);
        let num_style = if bad { Style::new().fg(BAD).bold() } else { Style::new().fg(DIM) };
        let text_style = if bad {
            Style::new().fg(BAD)
        } else if cut {
            Style::new().fg(DIM)
        } else {
            Style::new()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{n:>4} "), num_style),
            Span::styled(l.to_string(), text_style),
        ]));
    }
    Text::from(lines)
}

fn projects_view(f: &mut Frame, area: Rect, app: &mut App) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);
    let w = left.width.saturating_sub(4) as usize;
    let rows: Vec<ListItem> = app
        .proj_rows
        .iter()
        .map(|&i| {
            let p = &app.projects[i];
            let (mark, st) = match (&p.real, p.exists) {
                (_, true) => ("✓", Style::new().fg(Color::Green)),
                (Some(_), false) => ("✗", Style::new().fg(BAD)),
                (None, false) => ("?", Style::new().fg(WARN)),
            };
            let mem = if p.mem_files > 0 {
                format!("{} mem {}", p.mem_files, fmt_bytes(p.mem_bytes))
            } else {
                String::new()
            };
            let right = format!("{mem:>14} {:>4}s {:>5}", p.sessions, fmt_age(age_days(p.newest)));
            let lw = w.saturating_sub(right.chars().count() + 3);
            let name = truncate(&show_project(p), lw);
            let pad = lw.saturating_sub(name.chars().count());
            ListItem::new(Line::from(vec![
                Span::styled(format!("{mark} "), st),
                Span::raw(name),
                Span::raw(" ".repeat(pad + 1)),
                Span::styled(right, Style::new().fg(DIM)),
            ]))
        })
        .collect();
    let gone = app.projects.iter().filter(|p| !p.exists).count();
    let title = format!(
        " All projects: {} ({} not on disk){} ",
        app.projects.len(),
        gone,
        if app.only_gone { " — gone only" } else { "" }
    );
    let list = List::new(rows)
        .block(Block::new().borders(Borders::ALL).title(title))
        .highlight_style(Style::new().bg(Color::Rgb(50, 50, 70)).add_modifier(Modifier::BOLD))
        .highlight_symbol("▌");
    f.render_stateful_widget(list, left, &mut app.proj_list);

    let block = Block::new().borders(Borders::ALL);
    let Some(p) = app.selected_project() else {
        f.render_widget(Paragraph::new("no projects").block(block), right);
        return;
    };
    let mut lines = vec![
        kv("path", p.real.as_ref().map(|r| tilde(r)).unwrap_or_else(|| "unknown".into())),
        kv(
            "status",
            match (&p.real, p.exists) {
                (_, true) => "directory exists".into(),
                (Some(_), false) => "directory is gone".into(),
                (None, false) => "directory not found on disk (no session records its path)".into(),
            },
        ),
        kv("data", tilde(&p.dir)),
        kv("sessions", p.sessions.to_string()),
        kv("memory", format!("{} files, {}", p.mem_files, fmt_bytes(p.mem_bytes))),
        kv("active", format!("{} ago", fmt_age(age_days(p.newest)))),
        Line::from(Span::styled("─".repeat(60), Style::new().fg(DIM))),
    ];
    match read(&p.memory_dir().join("MEMORY.md")) {
        Some(c) => lines.extend(c.lines().map(|l| Line::raw(l.to_string()))),
        None => lines.push(Line::from(Span::styled("no MEMORY.md", Style::new().fg(DIM)))),
    }
    let para = Paragraph::new(lines)
        .block(block.title(" MEMORY.md "))
        .wrap(Wrap { trim: false })
        .scroll((app.preview_scroll, 0));
    f.render_widget(para, right);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let [v] = Layout::vertical([Constraint::Length(h.min(area.height))]).flex(Flex::Center).areas(area);
    let [r] = Layout::horizontal([Constraint::Length(w.min(area.width))]).flex(Flex::Center).areas(v);
    r
}

fn modal(f: &mut Frame, app: &App) {
    match &app.modal {
        Modal::None => {}
        Modal::Confirm { msg, .. } => {
            let w = msg.iter().map(|m| m.chars().count()).max().unwrap_or(20) as u16 + 6;
            let area = centered(f.area(), w.max(40), msg.len() as u16 + 4);
            f.render_widget(Clear, area);
            let mut lines: Vec<Line> = msg.iter().map(|m| Line::raw(m.clone())).collect();
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled("y", Style::new().fg(Color::Green).bold()),
                Span::raw(" confirm   any other key cancels"),
            ]));
            f.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .block(Block::new().borders(Borders::ALL).title(" Confirm ").border_style(Style::new().fg(WARN))),
                area,
            );
        }
        Modal::Help => {
            let area = centered(f.area(), 76, 30);
            f.render_widget(Clear, area);
            let help = [
                ("", "MARKERS"),
                ("●", "loads in full at every session start"),
                ("◐", "only name + description load; body loads on use"),
                ("○", "loads on demand (memory files, subdir CLAUDE.md, path rules)"),
                ("✗", "skill turned off in skillOverrides"),
                ("·", "not loaded as text (settings, inactive styles)"),
                ("!n", "n flags: missing paths, broken index links, old files"),
                ("", ""),
                ("", "CONTEXT VIEW"),
                ("j/k ↑/↓", "move;  g/G first/last;  PgUp/PgDn"),
                ("J/K ^d/^u", "scroll the preview"),
                ("e, enter", "open in $VISUAL / $EDITOR"),
                ("d", "move to trash (memory: also drops the index line)"),
                ("r", "review with `claude -p` (read-only tools), then diff"),
                ("s", "skill on/off via skillOverrides in user settings"),
                ("f", "show flagged items only"),
                ("p", "all projects view;  c  back to start directory"),
                ("R", "rescan"),
                ("", ""),
                ("", "REVIEW DIFF"),
                ("a / y", "accept: write file, old version to trash"),
                ("n / esc", "reject"),
                ("", ""),
                ("", "PROJECTS VIEW"),
                ("enter", "show the context of that project"),
                ("d / D", "trash memory dir / trash all data of project"),
            ];
            let lines: Vec<Line> = help
                .iter()
                .map(|(k, v)| {
                    if k.is_empty() {
                        Line::from(Span::styled(v.to_string(), Style::new().fg(Color::Magenta).bold()))
                    } else {
                        Line::from(vec![
                            Span::styled(format!("  {k:<12}"), Style::new().fg(Color::Cyan)),
                            Span::raw(v.to_string()),
                        ])
                    }
                })
                .collect();
            f.render_widget(
                Paragraph::new(lines).block(Block::new().borders(Borders::ALL).title(" Help — any key closes ")),
                area,
            );
        }
        Modal::Review { review, scroll } => {
            let a = f.area();
            let area = centered(a, a.width.saturating_sub(4), a.height.saturating_sub(2));
            f.render_widget(Clear, area);
            let mut lines = vec![Line::from(Span::styled("NOTES", Style::new().fg(Color::Magenta).bold()))];
            for l in review.notes.lines() {
                lines.push(Line::raw(l.to_string()));
            }
            let (old_t, new_t) = (tokens(&review.original), tokens(&review.revised));
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled("DIFF  ", Style::new().fg(Color::Magenta).bold()),
                Span::styled(
                    format!("≈{} → ≈{} tok", fmt_tokens(old_t), fmt_tokens(new_t)),
                    Style::new().fg(DIM),
                ),
            ]));
            if review.original == review.revised {
                lines.push(Line::from(Span::styled("no changes", Style::new().fg(Color::Green))));
            }
            let diff = TextDiff::from_lines(&review.original, &review.revised);
            for group in diff.grouped_ops(3) {
                lines.push(Line::from(Span::styled("  ⋯", Style::new().fg(DIM))));
                for op in group {
                    for ch in diff.iter_changes(&op) {
                        let (sign, st) = match ch.tag() {
                            ChangeTag::Delete => ("-", Style::new().fg(BAD)),
                            ChangeTag::Insert => ("+", Style::new().fg(Color::Green)),
                            ChangeTag::Equal => (" ", Style::new().fg(DIM)),
                        };
                        let text = ch.value().trim_end_matches('\n').to_string();
                        let row = if text.is_empty() { sign.trim().to_string() } else { format!("{sign} {text}") };
                        lines.push(Line::from(Span::styled(row, st)));
                    }
                }
            }
            f.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .scroll((*scroll, 0))
                    .block(
                        Block::new()
                            .borders(Borders::ALL)
                            .title(format!(" Review: {} ", tilde(&review.path)))
                            .title_bottom(" a accept (old version to trash)   n/esc reject   j/k scroll "),
                    ),
                area,
            );
        }
    }
}
