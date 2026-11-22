//! Prints unsafe code metrics for the tmux-rs workspace and maintains the
//! per-file unsafe limits.
//!
//! Usage:
//!
//! - `tmux-metrics`: totals and the 30 files with the most unsafe code.
//! - `tmux-metrics false-safe`: lists the false safe functions.
//! - `tmux-metrics limits [--allow-raise]`: rewrites the limit file from
//!   the current counts. Refuses to raise a limit unless told to.

use std::fs;
use std::process::ExitCode;

use tmux_metrics::*;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = workspace_root();
    let stats = match scan(&root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("scan failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let exceptions = read_exceptions(&root);

    match args.first().map(String::as_str) {
        None => {
            print_summary(&root, &stats, &exceptions);
            ExitCode::SUCCESS
        }
        Some("false-safe") => {
            for (path, f) in false_safe(&stats, &exceptions) {
                println!("{path}:{} {}", f.line, f.key);
            }
            ExitCode::SUCCESS
        }
        Some("limits") => update_limits(&root, &stats, args.iter().any(|a| a == "--allow-raise")),
        Some(other) => {
            eprintln!("unknown command {other}, expected false-safe or limits");
            ExitCode::FAILURE
        }
    }
}

fn print_summary(root: &std::path::Path, stats: &[FileStats], exceptions: &[String]) {
    let t = totals(root, stats, exceptions);
    println!("{} files, {} lines", t.files, t.lines);
    println!(
        "unsafe fn            {:>6} of {} functions",
        t.unsafe_fns, t.fns
    );
    println!("unsafe keywords      {:>6}", t.unsafe_tokens);
    println!("*mut                 {:>6}", t.mut_ptrs);
    println!("static mut           {:>6}", t.static_muts);
    println!(
        "forbid(unsafe_code)  {:>6} lines in {} files",
        t.forbid_lines, t.forbid_files
    );
    println!("false safe fns       {:>6}", t.false_safe);
    println!("bridge calls         {:>6}", t.bridge_calls);
    println!();

    let mut worst: Vec<&FileStats> = stats.iter().filter(|s| s.unsafe_tokens > 0).collect();
    worst.sort_by(|a, b| {
        b.unsafe_tokens
            .cmp(&a.unsafe_tokens)
            .then(a.path.cmp(&b.path))
    });
    println!(
        "{:>7} {:>6} {:>7} {:>6}  file",
        "unsafe", "*mut", "static", "lines"
    );
    for s in worst.iter().take(30) {
        println!(
            "{:>7} {:>6} {:>7} {:>6}  {}",
            s.unsafe_tokens, s.mut_ptrs, s.static_muts, s.lines, s.path
        );
    }
}

fn update_limits(root: &std::path::Path, stats: &[FileStats], allow_raise: bool) -> ExitCode {
    let old = read_limits(root).unwrap_or_default();
    if !old.is_empty() && !allow_raise {
        let over = over_limit(stats, &old);
        if !over.is_empty() {
            for (path, n, limit) in over {
                eprintln!("{path}: {n} unsafe, limit {limit}");
            }
            eprintln!(
                "refusing to raise limits, pass --allow-raise and give a reason in the commit"
            );
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = fs::write(root.join(LIMITS_FILE), render_limits(stats)) {
        eprintln!("cannot write {LIMITS_FILE}: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
