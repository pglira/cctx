use crate::actions;
use crate::model::*;
use crate::projects::{self, Project};
use crate::review::{self, Review};
use crate::scan::{self, Ctx};
use crate::ui;
use crate::util::tilde;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use ratatui::DefaultTerminal;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum View {
    Main,
    Projects,
}

pub enum Pending {
    TrashItem(usize),
    TrashProjectMemory(usize),
    TrashProject(usize),
}

pub enum Modal {
    None,
    Confirm { msg: Vec<String>, action: Pending },
    Review { review: Review, scroll: u16 },
    Help,
}

pub enum Row {
    Header(Group),
    Item(usize),
}

pub struct App {
    start_cwd: PathBuf,
    stale_days: u64,
    mem_override: Option<PathBuf>,
    pub viewing_project: Option<String>,
    pub ctx: Ctx,
    pub items: Vec<Item>,
    pub rows: Vec<Row>,
    pub list: ListState,
    pub preview_scroll: u16,
    pub only_flagged: bool,
    pub view: View,
    pub projects: Vec<Project>,
    pub proj_rows: Vec<usize>,
    pub proj_list: ListState,
    pub only_gone: bool,
    pub modal: Modal,
    pub status: String,
    rx: Option<Receiver<Result<Review, String>>>,
    pub reviewing: Option<(String, Instant)>,
    quit: bool,
}

impl App {
    pub fn new(cwd: PathBuf, stale_days: u64) -> App {
        let ctx = Ctx::new(cwd.clone(), None, stale_days);
        let mut app = App {
            start_cwd: cwd,
            stale_days,
            mem_override: None,
            viewing_project: None,
            ctx,
            items: Vec::new(),
            rows: Vec::new(),
            list: ListState::default(),
            preview_scroll: 0,
            only_flagged: false,
            view: View::Main,
            projects: Vec::new(),
            proj_rows: Vec::new(),
            proj_list: ListState::default(),
            only_gone: false,
            modal: Modal::None,
            status: "? help".into(),
            rx: None,
            reviewing: None,
            quit: false,
        };
        app.rescan();
        app
    }

    // ---- data ----

    fn set_target(&mut self, cwd: PathBuf, mem_override: Option<PathBuf>) {
        self.mem_override = mem_override;
        self.ctx = Ctx::new(cwd, self.mem_override.clone(), self.stale_days);
        self.list.select(None);
        self.rescan();
    }

    pub fn rescan(&mut self) {
        let keep = self.selected().map(|i| i.path.clone());
        self.ctx = Ctx::new(self.ctx.cwd.clone(), self.mem_override.clone(), self.stale_days);
        self.items = scan::scan(&self.ctx);
        self.rebuild_rows(keep);
    }

    fn rebuild_rows(&mut self, keep: Option<PathBuf>) {
        self.rows.clear();
        let mut groups: Vec<Group> = self.items.iter().map(|i| i.group).collect();
        groups.sort();
        groups.dedup();
        for g in groups {
            let idx: Vec<usize> = (0..self.items.len())
                .filter(|&i| self.items[i].group == g)
                .filter(|&i| !self.only_flagged || !self.items[i].flags.is_empty())
                .collect();
            if idx.is_empty() {
                continue;
            }
            self.rows.push(Row::Header(g));
            self.rows.extend(idx.into_iter().map(Row::Item));
        }
        let pos = keep
            .and_then(|p| {
                self.rows
                    .iter()
                    .position(|r| matches!(r, Row::Item(i) if self.items[*i].path == p))
            })
            .or_else(|| self.list.selected().filter(|&s| matches!(self.rows.get(s), Some(Row::Item(_)))))
            .or_else(|| self.rows.iter().position(|r| matches!(r, Row::Item(_))));
        self.list.select(pos);
    }

    pub fn selected_index(&self) -> Option<usize> {
        match self.rows.get(self.list.selected()?)? {
            Row::Item(i) => Some(*i),
            Row::Header(_) => None,
        }
    }

    pub fn selected(&self) -> Option<&Item> {
        self.selected_index().map(|i| &self.items[i])
    }

    pub fn group_tokens(&self, g: Group) -> usize {
        self.items.iter().filter(|i| i.group == g).map(|i| i.tokens_loaded).sum()
    }

    pub fn total_tokens(&self) -> usize {
        self.items.iter().map(|i| i.tokens_loaded).sum()
    }

    pub fn flagged_count(&self) -> usize {
        self.items.iter().filter(|i| !i.flags.is_empty()).count()
    }

    fn move_sel(&mut self, delta: i32) {
        let items: Vec<usize> = (0..self.rows.len())
            .filter(|&r| matches!(self.rows[r], Row::Item(_)))
            .collect();
        if items.is_empty() {
            return;
        }
        let cur = self.list.selected().and_then(|s| items.iter().position(|&r| r == s)).unwrap_or(0);
        let next = (cur as i32 + delta).clamp(0, items.len() as i32 - 1) as usize;
        self.list.select(Some(items[next]));
        self.preview_scroll = 0;
    }

    fn rebuild_proj_rows(&mut self) {
        self.proj_rows = (0..self.projects.len())
            .filter(|&i| !self.only_gone || !self.projects[i].exists)
            .collect();
        let sel = self.proj_list.selected().unwrap_or(0);
        self.proj_list
            .select(if self.proj_rows.is_empty() { None } else { Some(sel.min(self.proj_rows.len() - 1)) });
    }

    pub fn selected_project(&self) -> Option<&Project> {
        self.proj_rows.get(self.proj_list.selected()?).map(|&i| &self.projects[i])
    }

    fn move_proj(&mut self, delta: i32) {
        if self.proj_rows.is_empty() {
            return;
        }
        let cur = self.proj_list.selected().unwrap_or(0) as i32;
        let next = (cur + delta).clamp(0, self.proj_rows.len() as i32 - 1) as usize;
        self.proj_list.select(Some(next));
        self.preview_scroll = 0;
    }

    // ---- loop ----

    pub fn run(&mut self, term: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            term.draw(|f| ui::draw(f, self))?;
            self.poll_review();
            if event::poll(Duration::from_millis(250))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        self.on_key(k, term)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn poll_review(&mut self) {
        let Some(rx) = &self.rx else { return };
        let Ok(res) = rx.try_recv() else { return };
        self.rx = None;
        let name = self.reviewing.take().map(|r| r.0).unwrap_or_default();
        match res {
            Ok(r) if r.revised == r.original => {
                self.status = format!("review of {name}: no changes proposed");
                self.modal = Modal::Review { review: r, scroll: 0 };
            }
            Ok(r) => {
                self.status = format!("review of {name} ready");
                self.modal = Modal::Review { review: r, scroll: 0 };
            }
            Err(e) => self.status = format!("review failed: {e}"),
        }
    }

    fn on_key(&mut self, k: KeyEvent, term: &mut DefaultTerminal) -> Result<()> {
        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return Ok(());
        }
        match &mut self.modal {
            Modal::None => {}
            Modal::Help => {
                self.modal = Modal::None;
                return Ok(());
            }
            Modal::Confirm { .. } => {
                if matches!(k.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                    if let Modal::Confirm { action, .. } = std::mem::replace(&mut self.modal, Modal::None) {
                        self.do_pending(action);
                    }
                } else {
                    self.modal = Modal::None;
                    self.status = "cancelled".into();
                }
                return Ok(());
            }
            Modal::Review { scroll, .. } => {
                match k.code {
                    KeyCode::Char('j') | KeyCode::Down => *scroll = scroll.saturating_add(1),
                    KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
                    KeyCode::PageDown | KeyCode::Char(' ') => *scroll = scroll.saturating_add(20),
                    KeyCode::PageUp => *scroll = scroll.saturating_sub(20),
                    KeyCode::Char('a') | KeyCode::Char('y') => {
                        if let Modal::Review { review, .. } = std::mem::replace(&mut self.modal, Modal::None) {
                            self.accept_review(review);
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char('r') | KeyCode::Esc | KeyCode::Char('q') => {
                        self.modal = Modal::None;
                        self.status = "review rejected; file unchanged".into();
                    }
                    _ => {}
                }
                return Ok(());
            }
        }
        match self.view {
            View::Main => self.key_main(k, term),
            View::Projects => self.key_projects(k, term),
        }
    }

    fn key_common(&mut self, k: &KeyEvent) -> bool {
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.modal = Modal::Help,
            KeyCode::Char('J') | KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.preview_scroll = self.preview_scroll.saturating_add(15)
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.preview_scroll = self.preview_scroll.saturating_sub(15)
            }
            KeyCode::Char('J') => self.preview_scroll = self.preview_scroll.saturating_add(3),
            KeyCode::Char('K') => self.preview_scroll = self.preview_scroll.saturating_sub(3),
            _ => return false,
        }
        true
    }

    fn key_main(&mut self, k: KeyEvent, term: &mut DefaultTerminal) -> Result<()> {
        if self.key_common(&k) {
            return Ok(());
        }
        match k.code {
            KeyCode::Esc => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::PageDown => self.move_sel(10),
            KeyCode::PageUp => self.move_sel(-10),
            KeyCode::Char('g') | KeyCode::Home => self.move_sel(-100_000),
            KeyCode::Char('G') | KeyCode::End => self.move_sel(100_000),
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(it) = self.selected() {
                    let p = it.path.clone();
                    let ro = it.read_only;
                    self.edit(&p, term)?;
                    if ro {
                        self.status = "note: plugin files are overwritten on plugin update".into();
                    }
                }
            }
            KeyCode::Char('d') => self.ask_trash(),
            KeyCode::Char('s') => self.toggle_skill(),
            KeyCode::Char('r') => self.start_review(),
            KeyCode::Char('f') => {
                self.only_flagged = !self.only_flagged;
                let keep = self.selected().map(|i| i.path.clone());
                self.rebuild_rows(keep);
                self.status = if self.only_flagged { "showing flagged items only" } else { "showing all items" }.into();
            }
            KeyCode::Char('R') => {
                self.rescan();
                self.status = "rescanned".into();
            }
            KeyCode::Char('p') => {
                self.projects = projects::scan();
                self.rebuild_proj_rows();
                self.view = View::Projects;
                self.preview_scroll = 0;
            }
            KeyCode::Char('c') => {
                self.viewing_project = None;
                self.set_target(self.start_cwd.clone(), None);
                self.status = format!("back to {}", tilde(&self.start_cwd));
            }
            _ => {}
        }
        Ok(())
    }

    fn key_projects(&mut self, k: KeyEvent, term: &mut DefaultTerminal) -> Result<()> {
        if self.key_common(&k) {
            return Ok(());
        }
        match k.code {
            KeyCode::Esc | KeyCode::Char('p') => {
                self.view = View::Main;
                self.preview_scroll = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_proj(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_proj(-1),
            KeyCode::PageDown => self.move_proj(10),
            KeyCode::PageUp => self.move_proj(-10),
            KeyCode::Char('g') | KeyCode::Home => self.move_proj(-100_000),
            KeyCode::Char('G') | KeyCode::End => self.move_proj(100_000),
            KeyCode::Char('f') => {
                self.only_gone = !self.only_gone;
                self.rebuild_proj_rows();
            }
            KeyCode::Char('R') => {
                self.projects = projects::scan();
                self.rebuild_proj_rows();
            }
            KeyCode::Char('e') => {
                if let Some(p) = self.selected_project() {
                    let idx = p.memory_dir().join("MEMORY.md");
                    if idx.is_file() {
                        self.edit(&idx, term)?;
                    }
                }
            }
            KeyCode::Enter => {
                if let Some(p) = self.selected_project().cloned() {
                    let cwd = p.real.clone().unwrap_or_else(|| self.start_cwd.clone());
                    self.viewing_project = Some(match &p.real {
                        Some(r) => tilde(r),
                        None => p.name.clone(),
                    });
                    self.set_target(cwd, Some(p.memory_dir()));
                    self.view = View::Main;
                    self.status = "press c to return to the start directory".into();
                }
            }
            KeyCode::Char('d') => {
                if let Some(&i) = self.proj_list.selected().and_then(|s| self.proj_rows.get(s)) {
                    let p = &self.projects[i];
                    if p.mem_files == 0 {
                        self.status = "project has no memory files".into();
                    } else {
                        self.modal = Modal::Confirm {
                            msg: vec![
                                format!("Move the memory directory of {} to the trash?", show_project(p)),
                                format!("{} files, {}", p.mem_files, crate::util::fmt_bytes(p.mem_bytes)),
                                "Session transcripts stay.".into(),
                            ],
                            action: Pending::TrashProjectMemory(i),
                        };
                    }
                }
            }
            KeyCode::Char('D') => {
                if let Some(&i) = self.proj_list.selected().and_then(|s| self.proj_rows.get(s)) {
                    let p = &self.projects[i];
                    self.modal = Modal::Confirm {
                        msg: vec![
                            format!("Move ALL Claude data of {} to the trash?", show_project(p)),
                            format!("{}", tilde(&p.dir)),
                            format!(
                                "{} sessions (resume history is lost), {} memory files",
                                p.sessions, p.mem_files
                            ),
                        ],
                        action: Pending::TrashProject(i),
                    };
                }
            }
            _ => {}
        }
        Ok(())
    }

    // ---- actions ----

    fn edit(&mut self, path: &std::path::Path, term: &mut DefaultTerminal) -> Result<()> {
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vi".into());
        ratatui::restore();
        let res = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{editor} \"$1\""))
            .arg("sh")
            .arg(path)
            .status();
        *term = ratatui::init();
        term.clear()?;
        match res {
            Ok(s) if s.success() => self.status = format!("edited {}", tilde(path)),
            Ok(s) => self.status = format!("editor exited with {s}"),
            Err(e) => self.status = format!("cannot start editor: {e}"),
        }
        self.rescan();
        if self.view == View::Projects {
            self.projects = projects::scan();
            self.rebuild_proj_rows();
        }
        Ok(())
    }

    fn ask_trash(&mut self) {
        let Some(i) = self.selected_index() else { return };
        let it = &self.items[i];
        if it.read_only {
            self.status = "plugin files are read-only here; disable the skill (s) or the plugin".into();
            return;
        }
        let msg = match &it.kind {
            Kind::Settings => {
                self.status = "settings files cannot be trashed here; edit them (e)".into();
                return;
            }
            Kind::MemoryFile => vec![
                format!("Move {} to the trash?", tilde(&it.path)),
                "Its line in MEMORY.md is removed too.".into(),
            ],
            Kind::Skill { .. } => vec![format!(
                "Move the skill directory {} to the trash?",
                tilde(it.path.parent().unwrap_or(&it.path))
            )],
            _ => vec![format!("Move {} to the trash?", tilde(&it.path))],
        };
        self.modal = Modal::Confirm { msg, action: Pending::TrashItem(i) };
    }

    fn do_pending(&mut self, action: Pending) {
        let res: Result<String> = (|| match action {
            Pending::TrashItem(i) => {
                let it = self.items[i].clone();
                match &it.kind {
                    Kind::Skill { .. } => {
                        let dir = it.path.parent().unwrap_or(&it.path);
                        actions::trash(dir)?;
                        Ok(format!("trashed {}", tilde(dir)))
                    }
                    Kind::MemoryFile => {
                        actions::trash(&it.path)?;
                        let name = it.path.file_name().unwrap().to_string_lossy().to_string();
                        let index = it.path.with_file_name("MEMORY.md");
                        let n = actions::unlink_from_index(&index, &name)?;
                        Ok(format!("trashed {name}; removed {n} index line(s)"))
                    }
                    _ => {
                        actions::trash(&it.path)?;
                        Ok(format!("trashed {}", tilde(&it.path)))
                    }
                }
            }
            Pending::TrashProjectMemory(i) => {
                let d = self.projects[i].memory_dir();
                actions::trash(&d)?;
                Ok(format!("trashed {}", tilde(&d)))
            }
            Pending::TrashProject(i) => {
                let d = self.projects[i].dir.clone();
                actions::trash(&d)?;
                Ok(format!("trashed {}", tilde(&d)))
            }
        })();
        self.status = match res {
            Ok(s) => s,
            Err(e) => format!("error: {e:#}"),
        };
        self.rescan();
        if self.view == View::Projects {
            self.projects = projects::scan();
            self.rebuild_proj_rows();
        }
    }

    fn toggle_skill(&mut self) {
        let Some(it) = self.selected() else { return };
        let Kind::Skill { key } = &it.kind else {
            self.status = "s toggles skills only".into();
            return;
        };
        let off = it.load != Load::Disabled;
        let key = key.clone();
        let settings = self.ctx.user_settings();
        match actions::set_skill_off(&settings, &key, off) {
            Ok(()) => {
                self.status = format!(
                    "{key}: {} in {}",
                    if off { "off" } else { "on" },
                    tilde(&settings)
                );
                self.rescan();
            }
            Err(e) => self.status = format!("error: {e:#}"),
        }
    }

    fn start_review(&mut self) {
        if self.rx.is_some() {
            self.status = "a review is already running".into();
            return;
        }
        let Some(it) = self.selected() else { return };
        if it.read_only {
            self.status = "plugin files are read-only here".into();
            return;
        }
        let (tx, rx) = channel();
        let name = it.label.clone();
        review::spawn(
            it.path.clone(),
            it.kind.role().to_string(),
            self.ctx.cwd.clone(),
            it.content.clone(),
            tx,
        );
        self.rx = Some(rx);
        self.reviewing = Some((name, Instant::now()));
    }

    fn accept_review(&mut self, r: Review) {
        let current = crate::util::read(&r.path).unwrap_or_default();
        if current != r.original {
            self.status = "file changed during the review; not written".into();
            return;
        }
        self.status = match actions::replace_with_backup(&r.path, &r.revised) {
            Ok(()) => format!("wrote {} (old version in trash)", tilde(&r.path)),
            Err(e) => format!("error: {e:#}"),
        };
        self.rescan();
    }
}

pub fn show_project(p: &Project) -> String {
    match &p.real {
        Some(r) => tilde(r),
        None => p.name.clone(),
    }
}
