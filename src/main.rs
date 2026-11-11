mod actions;
mod app;
mod model;
mod projects;
mod review;
mod scan;
mod stale;
mod ui;
mod util;

use std::path::PathBuf;

const HELP: &str = "cctx — inspect and prune the Claude Code context of a directory

usage: cctx [options] [DIR]

DIR defaults to the current directory. cctx shows what a Claude Code session
started in DIR loads: CLAUDE.md files and imports, rules, auto memory, skills,
commands, agents, output styles, and settings.

options:
  --list             print the items as text and exit
  --projects         print all per-project data directories and exit
  --stale-days N     flag files not changed for N days (default 90)
  -h, --help         show this help
";

fn main() -> anyhow::Result<()> {
    // Exit quietly when stdout closes early, as in `cctx --list | head`.
    #[cfg(unix)]
    unsafe {
        libc_signal_default();
    }
    let mut args = std::env::args().skip(1);
    let mut dir: Option<PathBuf> = None;
    let mut stale_days = 90;
    let mut list = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            "--list" => list = true,
            "--projects" => {
                print_projects();
                return Ok(());
            }
            "--stale-days" => {
                stale_days = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("--stale-days needs a number"))?
            }
            _ if a.starts_with('-') => anyhow::bail!("unknown option {a}; see --help"),
            _ => dir = Some(PathBuf::from(a)),
        }
    }
    let cwd = match dir {
        Some(d) => std::fs::canonicalize(&d)?,
        None => std::env::current_dir()?,
    };
    if list {
        print_list(cwd, stale_days);
        return Ok(());
    }
    let mut app = app::App::new(cwd, stale_days);
    let mut terminal = ratatui::init();
    let res = app.run(&mut terminal);
    ratatui::restore();
    res
}

fn print_list(cwd: PathBuf, stale_days: u64) {
    use util::*;
    let ctx = scan::Ctx::new(cwd, None, stale_days);
    let items = scan::scan(&ctx);
    let total: usize = items.iter().map(|i| i.tokens_loaded).sum();
    println!("dir: {}   memory: {}", tilde(&ctx.cwd), tilde(&ctx.memory_dir));
    println!("loaded at start ≈ {} tok\n", fmt_tokens(total));
    let mut group = None;
    for it in &items {
        if group != Some(it.group) {
            group = Some(it.group);
            println!("{}", it.group.title().to_uppercase());
        }
        println!(
            "  {} {:<50} {:>7} {:>6} {:>5}",
            it.load.marker(),
            truncate(&format!("{}{} [{}]", "  ".repeat(it.depth), it.label, it.origin), 50),
            fmt_tokens(it.tokens_loaded),
            format!("({})", fmt_tokens(it.tokens_full)),
            fmt_age(age_days(it.mtime)),
        );
        for f in &it.flags {
            println!("      ! {}", f.text());
        }
    }
}

fn print_projects() {
    use util::*;
    for p in projects::scan() {
        let mark = match (&p.real, p.exists) {
            (_, true) => "ok  ",
            (Some(_), false) => "GONE",
            (None, false) => "??  ",
        };
        println!(
            "{mark} {:<60} {:>3} mem {:>7} {:>4} sessions {:>5}",
            truncate(&app::show_project(&p), 60),
            p.mem_files,
            fmt_bytes(p.mem_bytes),
            p.sessions,
            fmt_age(age_days(p.newest)),
        );
    }
}

/// Restore the default SIGPIPE action, which the Rust runtime sets to ignore.
#[cfg(unix)]
unsafe fn libc_signal_default() {
    extern "C" {
        fn signal(sig: i32, handler: usize) -> usize;
    }
    const SIGPIPE: i32 = 13;
    signal(SIGPIPE, 0);
}
