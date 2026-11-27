//! `cargo xtask format-diff`: expands formats on a tmux-rs server and a
//! tmux 3.8 server set up the same way and reports every difference.
//!
//! Both servers run on private sockets under /tmp/fdiff-PID with
//! `-f/dev/null` and the same fixed environment, and get the same
//! sessions, windows, panes (each running `sleep 1000`), options,
//! environment and paste buffer (see [`setup_commands`]). Each format is
//! expanded with `display-message -t TARGET -p -- FORMAT`, the format as
//! one argument, on both sides at once. Standard output and the exit
//! status are compared byte for byte. A server that dies or a command
//! that times out is a difference of its own, and that server is started
//! again before the run goes on.
//!
//! The formats come from the corpus file (see [`corpus`]) and from the
//! generator (see [`generate`]). A differing format is reduced to its
//! smallest `#{...}` part that still differs and grouped by kind (see
//! [`group`]). A file of known differences (see [`known`]) turns the run
//! into a gate: the exit status is non-zero only for differences it does
//! not list.

pub mod corpus;
pub mod generate;
pub mod group;
pub mod known;
pub mod server;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use server::{Outcome, Side, Status};

/// The usage text of the subcommand.
pub const USAGE: &str = "usage: cargo xtask format-diff [options]

Expands formats on a tmux-rs server and a tmux 3.8 server set up the
same way and reports every difference.

options:
  --bin PATH          tmux-rs binary (default target/debug/tmux-rs)
  --reference PATH    tmux 3.8 binary (default /usr/bin/tmux)
  --corpus PATH       corpus file (default xtask/format-corpus.txt)
  --corpus-only       check only the corpus, generate nothing
  --seed N            seed for generated formats (default 1)
  --count N           number of generated formats (default 1000)
  --format F          check only this format (may be repeated)
  --target T          target of display-message (default alpha:0.1)
  --timeout MS        time limit per command in ms (default 5000)
  --known PATH        file of known differences, which then do not fail
  --write-known PATH  write a known differences file with a group entry
                      for every group that differs, keeping the entries
                      of --known
  --no-reduce         do not reduce differing formats to their smallest
                      differing part
  --all               print every difference, not one per group";

/// Command line options.
#[derive(Debug, Clone)]
pub struct Options {
    /// The tmux-rs binary.
    pub bin: PathBuf,
    /// The tmux 3.8 binary.
    pub reference: PathBuf,
    /// The corpus file.
    pub corpus: PathBuf,
    /// Check only the corpus.
    pub corpus_only: bool,
    /// Seed of the generator.
    pub seed: u64,
    /// How many formats to generate.
    pub count: usize,
    /// Formats from the command line, which replace corpus and generator.
    pub formats: Vec<String>,
    /// Target pane of display-message.
    pub target: String,
    /// Time limit per command.
    pub timeout: Duration,
    /// Known differences file.
    pub known: Option<PathBuf>,
    /// Where to write a known differences file.
    pub write_known: Option<PathBuf>,
    /// Reduce differing formats.
    pub reduce: bool,
    /// Print every difference.
    pub all: bool,
}

impl Options {
    /// Parses the arguments after `format-diff`.
    pub fn parse(args: &[String], root: &std::path::Path) -> Result<Options, String> {
        let mut o = Options {
            bin: root.join("target/debug/tmux-rs"),
            reference: PathBuf::from("/usr/bin/tmux"),
            corpus: root.join("xtask/format-corpus.txt"),
            corpus_only: false,
            seed: 1,
            count: 1000,
            formats: Vec::new(),
            target: "alpha:0.1".to_string(),
            timeout: Duration::from_millis(5000),
            known: None,
            write_known: None,
            reduce: true,
            all: false,
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut value = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| format!("{a} needs a value"))
            };
            match a.as_str() {
                "--bin" => o.bin = value()?.into(),
                "--reference" => o.reference = value()?.into(),
                "--corpus" => o.corpus = value()?.into(),
                "--corpus-only" => o.corpus_only = true,
                "--seed" => o.seed = number(&value()?)?,
                "--count" => o.count = number(&value()?)? as usize,
                "--format" => o.formats.push(value()?),
                "--target" => o.target = value()?,
                "--timeout" => o.timeout = Duration::from_millis(number(&value()?)?),
                "--known" => o.known = Some(value()?.into()),
                "--write-known" => o.write_known = Some(value()?.into()),
                "--no-reduce" => o.reduce = false,
                "--all" => o.all = true,
                "-h" | "--help" => return Err(String::new()),
                _ => return Err(format!("unknown option {a}")),
            }
        }
        Ok(o)
    }
}

/// Parses a number option.
fn number(s: &str) -> Result<u64, String> {
    s.parse().map_err(|_| format!("not a number: {s}"))
}

/// The commands that set a server up, in order. The first one starts it.
/// `home` is the directory panes start in.
pub fn setup_commands(home: &str) -> Vec<Vec<String>> {
    let lines: &[&[&str]] = &[
        &[
            "new-session",
            "-d",
            "-s",
            "alpha",
            "-n",
            "editor",
            "-x",
            "80",
            "-y",
            "24",
            "-c",
            home,
            "sleep 1000",
        ],
        &[
            "split-window",
            "-d",
            "-h",
            "-t",
            "alpha:0",
            "-c",
            home,
            "sleep 1000",
        ],
        &[
            "split-window",
            "-d",
            "-v",
            "-l",
            "5",
            "-t",
            "alpha:0.0",
            "-c",
            home,
            "sleep 1000",
        ],
        &[
            "new-window",
            "-d",
            "-t",
            "alpha:3",
            "-n",
            "logs",
            "-c",
            home,
            "sleep 1000",
        ],
        &[
            "new-window",
            "-d",
            "-t",
            "alpha:5",
            "-n",
            "café ☕",
            "-c",
            home,
            "sleep 1000",
        ],
        &[
            "new-session",
            "-d",
            "-s",
            "beta",
            "-n",
            "main",
            "-x",
            "100",
            "-y",
            "30",
            "-c",
            home,
            "sleep 1000",
        ],
        &["new-session", "-d", "-s", "gamma", "-t", "alpha"],
        &[
            "new-session",
            "-d",
            "-s",
            "δelta",
            "-n",
            "one",
            "-c",
            home,
            "sleep 1000",
        ],
        &["select-pane", "-t", "alpha:0.0", "-T", "left pane"],
        &["select-pane", "-t", "alpha:0.1", "-T", "right, {pane}"],
        &["select-pane", "-t", "alpha:0.2", "-T", "bottom"],
        &["select-pane", "-t", "alpha:3.0", "-T", "logs pane"],
        &["select-pane", "-t", "alpha:5.0", "-T", "tïtle ☕"],
        &["select-pane", "-t", "beta:0.0", "-T", "beta pane"],
        &["select-pane", "-t", "δelta:0.0", "-T", "delta pane"],
        &["select-pane", "-t", "alpha:0.1"],
        &["set", "-g", "@plain", "hello"],
        &["set", "-g", "@num", "42"],
        &["set", "-g", "@neg", "-7"],
        &["set", "-g", "@float", "3.25"],
        &["set", "-g", "@path", "/usr/local/lib/libfoo.so.1"],
        &["set", "-g", "@dir", "/usr/local/"],
        &["set", "-g", "@utf8", "héllo wörld ☕ 日本"],
        &["set", "-g", "@empty", ""],
        &["set", "-g", "@dotdot", "a/.."],
        &["set", "-g", "@fmt", "#{session_name}:#{window_index}"],
        &["set", "-g", "@punct", "a,b{c}d#e"],
        &["set", "-g", "@hashbrace", "#{?1,yes,no}"],
        &[
            "set",
            "-g",
            "@long",
            "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ",
        ],
        &["set", "-g", "@spaces", "  padded  "],
        &["set", "-g", "@shell", "it's \"quoted\" $HOME;&|<>*?"],
        &["set", "-g", "@colour", "red"],
        &["set", "-g", "@true", "1"],
        &["set", "-g", "@false", "0"],
        &["set", "-g", "@zero", "00"],
        &["set", "-w", "-t", "alpha:0", "@wopt", "window-value"],
        &["set", "-w", "-t", "alpha:3", "@wopt", "other-window"],
        &["set", "-p", "-t", "alpha:0.1", "@popt", "pane-value"],
        &["set", "-t", "beta", "@sopt", "session-value"],
        &["set", "-t", "alpha", "@sopt", "alpha-value"],
        &["setenv", "-g", "FOO", "bar"],
        &["setenv", "-g", "EMPTYENV", ""],
        &["setenv", "-t", "alpha", "SESSVAR", "local"],
        &["setenv", "-g", "-r", "REMOVED"],
        &["set-buffer", "-b", "buf0", "paste, data {x} #{y}"],
        &["set-buffer", "-b", "buf1", "second buffer"],
    ];
    lines
        .iter()
        .map(|l| l.iter().map(|s| s.to_string()).collect())
        .collect()
}

/// What comparing one format gave.
#[derive(Debug, Clone)]
enum Verdict {
    /// Same output and status.
    Equal,
    /// Different output or status.
    Differ { rs: Outcome, c: Outcome },
    /// A server died. `side` is its name, `panic` its panic files.
    Crash {
        side: String,
        panic: Vec<String>,
        rs: Outcome,
        c: Outcome,
    },
    /// A command timed out on `side`.
    Timeout {
        side: String,
        rs: Outcome,
        c: Outcome,
    },
}

/// The kind of a difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Crash,
    Timeout,
    Output,
}

/// One difference found.
#[derive(Debug, Clone)]
struct Difference {
    kind: Kind,
    /// The format that was checked.
    format: String,
    /// Its smallest part that differs too, when there is one.
    reduced: Option<String>,
    /// The group it belongs to.
    group: String,
    /// Outputs of the smallest differing format.
    rs: Outcome,
    c: Outcome,
    /// For crashes and timeouts, which side and the panic text.
    detail: String,
    /// The known entry that covers it.
    known: Option<usize>,
}

/// Counts for the summary.
#[derive(Debug, Default)]
struct Counts {
    checked: usize,
    skipped: usize,
    equal: usize,
    different: usize,
    crashes: usize,
    timeouts: usize,
}

/// The two sides and the settings of a run.
struct Run {
    rs: Side,
    c: Side,
    setup: Vec<Vec<String>>,
    target: String,
    timeout: Duration,
    restarts: usize,
}

impl Run {
    /// Expands a format on both sides at once and compares.
    fn check(&mut self, format: &str) -> Verdict {
        let args = [
            "display-message",
            "-t",
            self.target.as_str(),
            "-p",
            "--",
            format,
        ];
        let timeout = self.timeout;
        let (rs, c) = thread::scope(|s| {
            let h = s.spawn(|| self.rs.run(&args, timeout));
            let c = self.c.run(&args, timeout);
            (h.join().expect("runner thread panicked"), c)
        });
        let rs_dead = !self.rs.alive();
        let c_dead = !self.c.alive();
        let rs_restart = rs_dead || rs.status == Status::Timeout;
        let c_restart = c_dead || c.status == Status::Timeout;
        let verdict = if rs_dead || c_dead {
            let mut side = Vec::new();
            let mut panic = Vec::new();
            if rs_dead {
                side.push(self.rs.name.clone());
                panic = self.rs.take_panic_files();
            }
            if c_dead {
                side.push(self.c.name.clone());
            }
            Verdict::Crash {
                side: side.join(" and "),
                panic,
                rs,
                c,
            }
        } else if rs.status == Status::Timeout || c.status == Status::Timeout {
            let mut side = Vec::new();
            if rs.status == Status::Timeout {
                side.push(self.rs.name.clone());
            }
            if c.status == Status::Timeout {
                side.push(self.c.name.clone());
            }
            Verdict::Timeout {
                side: side.join(" and "),
                rs,
                c,
            }
        } else if rs.status == c.status && rs.stdout == c.stdout {
            Verdict::Equal
        } else {
            Verdict::Differ { rs, c }
        };
        self.restart(rs_restart, c_restart);
        verdict
    }

    /// Starts the sides that need it again.
    fn restart(&mut self, rs: bool, c: bool) {
        for (want, side) in [(rs, &mut self.rs), (c, &mut self.c)] {
            if want {
                self.restarts += 1;
                if let Err(e) = side.start(&self.setup, self.timeout) {
                    eprintln!("format-diff: restart failed: {e}");
                }
            }
        }
    }

    /// Checks a format and turns the verdict into a difference, reducing
    /// output differences to their smallest differing part.
    fn compare(&mut self, format: &str, reduce: bool, found: &mut Vec<Difference>) -> bool {
        match self.check(format) {
            Verdict::Equal => true,
            Verdict::Differ { rs, c } => {
                let mut diff = Difference {
                    kind: Kind::Output,
                    format: format.to_string(),
                    reduced: None,
                    group: group::classify(format),
                    rs,
                    c,
                    detail: String::new(),
                    known: None,
                };
                if reduce {
                    // Each part on its own, then each part followed by
                    // text: 3.8 drops the rest of a format after some
                    // errors, which only shows when something follows.
                    let parts: Vec<String> = group::subterms(format).into_iter().take(24).collect();
                    let candidates = parts
                        .iter()
                        .map(|p| (p.clone(), p.clone()))
                        .chain(parts.iter().map(|p| (format!("{p}x"), p.clone())));
                    for (part, kind) in candidates {
                        match self.check(&part) {
                            Verdict::Equal => continue,
                            Verdict::Differ { rs, c } => {
                                diff.group = group::classify(&kind);
                                diff.reduced = Some(part);
                                diff.rs = rs;
                                diff.c = c;
                            }
                            other => found.push(failure(&part, other)),
                        }
                        break;
                    }
                }
                diff.group = group::refine(diff.group, &diff.rs.stdout, &diff.c.stdout);
                // A modifier applied to a variable that differs by itself
                // goes with the variable.
                let smallest = diff.reduced.as_deref().unwrap_or(format);
                if let Some(name) = group::plain_subject(smallest) {
                    let var = format!("#{{{name}}}");
                    if let Verdict::Differ { rs, c } = self.check(&var) {
                        diff.group = group::refine(group::classify(&var), &rs.stdout, &c.stdout);
                    }
                }
                found.push(diff);
                false
            }
            other => {
                let crashed = matches!(other, Verdict::Crash { .. });
                let mut diff = failure(format, other);
                if reduce && crashed {
                    // The shortest part that crashes on its own.
                    for part in group::subterms(format).into_iter().take(24) {
                        let v = self.check(&part);
                        if matches!(v, Verdict::Crash { .. }) {
                            let smaller = failure(&part, v);
                            diff.group = smaller.group;
                            diff.detail = smaller.detail;
                            diff.reduced = Some(part);
                            break;
                        }
                    }
                }
                found.push(diff);
                false
            }
        }
    }
}

/// Turns a crash or timeout verdict into a difference.
fn failure(format: &str, v: Verdict) -> Difference {
    let (kind, detail, rs, c) = match v {
        Verdict::Crash { side, panic, rs, c } => {
            let mut d = format!("{side} server died");
            for p in panic {
                let _ = write!(d, "\n    {}", panic_summary(&p));
            }
            (Kind::Crash, d, rs, c)
        }
        Verdict::Timeout { side, rs, c } => (Kind::Timeout, format!("{side} timed out"), rs, c),
        Verdict::Equal | Verdict::Differ { .. } => unreachable!("not a failure"),
    };
    let group = format!(
        "{}: {}",
        if kind == Kind::Crash {
            "crash"
        } else {
            "timeout"
        },
        group::classify(format)
    );
    Difference {
        kind,
        format: format.to_string(),
        reduced: None,
        group,
        rs,
        c,
        detail,
        known: None,
    }
}

/// The first line of a panic file and the first tmux-rs frame of its
/// backtrace after the panic hook.
fn panic_summary(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    let frame = text
        .lines()
        .find(|l| l.contains("fn: \"tmux_rs::") && !l.contains("server_start"))
        .map(|l| l.trim().trim_end_matches(',').to_string());
    match frame {
        Some(f) => format!("{first}\n    at {f}"),
        None => first.to_string(),
    }
}

/// Shows bytes as text: valid UTF-8 as it is apart from control
/// characters, which are escaped like the rest of the invalid bytes.
/// Long outputs are cut unless `full`.
fn show(b: &[u8], full: bool) -> String {
    const MAX: usize = 300;
    let mut s = String::from("\"");
    for chunk in b.utf8_chunks() {
        for ch in chunk.valid().chars() {
            match ch {
                '"' => s.push_str("\\\""),
                '\\' => s.push_str("\\\\"),
                c if c.is_control() => s.extend(c.escape_default()),
                c => s.push(c),
            }
        }
        for byte in chunk.invalid() {
            let _ = write!(s, "\\x{byte:02x}");
        }
    }
    s.push('"');
    if !full && s.chars().count() > MAX {
        let cut: String = s.chars().take(MAX).collect();
        return format!("{cut}... ({} bytes)", b.len());
    }
    s
}

/// "1 format", "2 formats".
fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("{n} {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// Shows one outcome on a line.
fn show_outcome(o: &Outcome, full: bool) -> String {
    let mut s = format!("{} {}", o.status, show(&o.stdout, full));
    let err = String::from_utf8_lossy(&o.stderr);
    let err = err.trim();
    if !err.is_empty() {
        let _ = write!(s, " stderr {}", show(err.as_bytes(), full));
    }
    s
}

/// Prints one difference.
fn print_difference(d: &Difference, rs_name: &str, c_name: &str, full: bool) {
    println!("    format:   {}", show(d.format.as_bytes(), true));
    if let Some(r) = &d.reduced {
        println!("    smallest: {}", show(r.as_bytes(), true));
    }
    if !d.detail.is_empty() {
        println!("    {}", d.detail);
    }
    println!("    {c_name:<9} {}", show_outcome(&d.c, full));
    println!("    {rs_name:<9} {}", show_outcome(&d.rs, full));
}

/// Runs the subcommand with the arguments after `format-diff`.
pub fn main(args: &[String], root: &std::path::Path) -> ExitCode {
    let opts = match Options::parse(args, root) {
        Ok(o) => o,
        Err(e) if e.is_empty() => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match run(&opts) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("format-diff: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Runs a whole comparison. Returns Ok(false) when a difference that is
/// not known was found.
fn run(opts: &Options) -> Result<bool, String> {
    let known_text = match &opts.known {
        Some(p) => {
            fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?
        }
        None => String::new(),
    };
    let known = known::parse(&known_text).map_err(|e| {
        let path = opts.known.as_deref().unwrap_or(Path::new(""));
        format!("bad lines in {}:\n{e}", path.display())
    })?;

    // The formats, as (source, format) with the source for the summary.
    let mut batches: Vec<(&str, Vec<String>)> = Vec::new();
    if !opts.formats.is_empty() {
        batches.push(("command line", opts.formats.clone()));
    } else {
        let text = fs::read_to_string(&opts.corpus)
            .map_err(|e| format!("cannot read {}: {e}", opts.corpus.display()))?;
        batches.push(("corpus", corpus::parse(&text)));
        if !opts.corpus_only && opts.count > 0 {
            let generated = generate::Generator::new(opts.seed).formats(opts.count);
            batches.push(("generated", generated));
        }
    }

    for bin in [&opts.bin, &opts.reference] {
        if !bin.exists() {
            return Err(format!("{} does not exist", bin.display()));
        }
    }

    let dir = PathBuf::from(format!("/tmp/fdiff-{}", std::process::id()));
    let home = dir.join("home");
    // Both servers start in the same directory, which becomes the path of
    // sessions created without -c. tmux-rs writes its panic files there.
    let work = dir.join("work");
    for d in [work.clone(), home.clone()] {
        fs::create_dir_all(&d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    }
    let home_s = home.to_string_lossy().into_owned();
    let env: Vec<(String, String)> = [
        ("PATH", "/usr/bin:/bin"),
        ("TERM", "screen"),
        ("SHELL", "/bin/sh"),
        ("HISTFILE", "/dev/null"),
        ("HOME", home_s.as_str()),
        ("TZ", "UTC"),
        ("LC_ALL", "C"),
        ("CLIENTVAR", "client-value"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    let mut run = Run {
        rs: Side::new(
            "tmux-rs",
            opts.bin.clone(),
            dir.join("rs.sock"),
            work.clone(),
            env.clone(),
        ),
        c: Side::new(
            "tmux 3.8",
            opts.reference.clone(),
            dir.join("c.sock"),
            work,
            env,
        ),
        setup: setup_commands(&home_s),
        target: opts.target.clone(),
        timeout: opts.timeout,
        restarts: 0,
    };
    let result = compare_all(&mut run, opts, &batches, &known);
    run.rs.stop();
    run.c.stop();
    let leftover = run.rs.take_panic_files();
    drop(run);
    let _ = fs::remove_dir_all(&dir);
    if !leftover.is_empty() {
        eprintln!(
            "format-diff: panic files left over:\n{}",
            leftover.join("\n")
        );
    }
    result
}

/// Sets both servers up, checks every batch and prints the report.
fn compare_all(
    run: &mut Run,
    opts: &Options,
    batches: &[(&str, Vec<String>)],
    known: &[known::Entry],
) -> Result<bool, String> {
    run.rs.start(&run.setup, run.timeout)?;
    run.c.start(&run.setup, run.timeout)?;
    let version = run
        .c
        .run(&["-V"], run.timeout)
        .stdout
        .into_iter()
        .filter(|&b| b != b'\n')
        .collect::<Vec<u8>>();
    println!(
        "format-diff: {} against {} ({}), target {}",
        opts.bin.display(),
        opts.reference.display(),
        String::from_utf8_lossy(&version),
        opts.target
    );

    let mut counts = Counts::default();
    let mut found: Vec<Difference> = Vec::new();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    for (source, formats) in batches {
        let start = Instant::now();
        let before = counts.checked;
        for f in formats {
            if let Some(why) = corpus::excluded(f) {
                counts.skipped += 1;
                *skipped.entry(why).or_default() += 1;
                continue;
            }
            counts.checked += 1;
            if run.compare(f, opts.reduce, &mut found) {
                counts.equal += 1;
            }
        }
        println!(
            "{source}: {} checked in {:.1}s",
            plural(counts.checked - before, "format"),
            start.elapsed().as_secs_f64()
        );
    }

    for d in &mut found {
        d.known = known::find(known, &d.group, &d.format);
        match d.kind {
            Kind::Crash => counts.crashes += 1,
            Kind::Timeout => counts.timeouts += 1,
            Kind::Output => counts.different += 1,
        }
    }
    // Reductions can add a crash for a format that also has an output
    // difference, so formats that are not equal are counted apart from
    // the differences.
    println!(
        "\nchecked {}, equal {}, not equal {}, output differences {}, crashes {}, timeouts {}, skipped {} (excluded), server restarts {}",
        counts.checked,
        counts.equal,
        counts.checked - counts.equal,
        counts.different,
        counts.crashes,
        counts.timeouts,
        counts.skipped,
        run.restarts
    );
    if !skipped.is_empty() {
        let list: Vec<String> = skipped.iter().map(|(k, n)| format!("{k} ({n})")).collect();
        println!("skipped because: {}", list.join(", "));
    }

    // Crashes and timeouts in full.
    for d in found.iter().filter(|d| d.kind != Kind::Output) {
        println!(
            "\n{}{}:",
            if d.kind == Kind::Crash {
                "CRASH"
            } else {
                "TIMEOUT"
            },
            if d.known.is_some() { " (known)" } else { "" }
        );
        print_difference(d, &run.rs.name, &run.c.name, opts.all);
    }

    // Output differences by group, largest first.
    let mut groups: BTreeMap<&str, Vec<&Difference>> = BTreeMap::new();
    for d in found.iter().filter(|d| d.kind == Kind::Output) {
        groups.entry(d.group.as_str()).or_default().push(d);
    }
    let mut ordered: Vec<(&str, Vec<&Difference>)> = groups.into_iter().collect();
    ordered.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
    if !ordered.is_empty() {
        println!("\ndifferences by kind:");
    }
    for (name, diffs) in &ordered {
        let new = diffs.iter().filter(|d| d.known.is_none()).count();
        let mark = if new == 0 {
            ", known".to_string()
        } else if new < diffs.len() {
            format!(", {new} new")
        } else {
            String::new()
        };
        println!("\n  {name} ({}{mark})", plural(diffs.len(), "format"));
        let shown = if opts.all { diffs.len() } else { 1 };
        for d in diffs.iter().take(shown) {
            print_difference(d, &run.rs.name, &run.c.name, opts.all);
        }
        // Without --all, the next few formats on a line each.
        const ALSO: usize = 8;
        if !opts.all && diffs.len() > 1 {
            println!("    also:");
            for d in diffs.iter().skip(1).take(ALSO) {
                let f = d.reduced.as_deref().unwrap_or(&d.format);
                println!("      {}", show(f.as_bytes(), false));
            }
            if diffs.len() > ALSO + 1 {
                println!("      and {} more", diffs.len() - ALSO - 1);
            }
        }
    }

    // Known entries: what matched nothing.
    let new = found.iter().filter(|d| d.known.is_none()).count();
    if opts.known.is_some() {
        let stale: Vec<&known::Entry> = known
            .iter()
            .enumerate()
            .filter(|(i, _)| !found.iter().any(|d| d.known == Some(*i)))
            .map(|(_, e)| e)
            .collect();
        println!(
            "\nknown differences: {} matched, {} new, {} entries matched nothing",
            found.len() - new,
            new,
            stale.len()
        );
        if !stale.is_empty() {
            print!(
                "entries that matched nothing (remove them if they are fixed):\n{}",
                known::render(&stale.into_iter().cloned().collect::<Vec<_>>())
            );
        }
    }

    if let Some(path) = &opts.write_known {
        let mut entries: Vec<known::Entry> = known.to_vec();
        // Crashes and timeouts first, then the output groups by size.
        let mut new_groups: Vec<&str> = found
            .iter()
            .filter(|d| d.kind != Kind::Output)
            .map(|d| d.group.as_str())
            .collect();
        new_groups.extend(ordered.iter().map(|(name, _)| *name));
        for name in new_groups {
            if known::find(&entries, name, "").is_none() {
                entries.push(known::Entry {
                    key: known::Key::Group(name.to_string()),
                    reason: "not triaged".to_string(),
                });
            }
        }
        // Keep the comments at the top of the --known file.
        let old = opts
            .known
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .unwrap_or_default();
        let text = known::header(&old).to_string() + &known::render(&entries);
        fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        println!("wrote {} entries to {}", entries.len(), path.display());
    }

    Ok(new == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_escapes() {
        assert_eq!(show(b"a\"b\\c\n\x1b", true), "\"a\\\"b\\\\c\\n\\u{1b}\"");
        assert_eq!(show("é☕".as_bytes(), true), "\"é☕\"");
        assert_eq!(show(b"\xff", true), "\"\\xff\"");
        assert!(show(&[b'a'; 1000], false).ends_with("(1000 bytes)"));
    }

    #[test]
    fn panic_lines() {
        let text = "panic! \"boom\"\nBacktrace [\n    { fn: \"tmux_rs::server::server_start::{closure#0}\", line: 1 },\n    { fn: \"std::x\" },\n    { fn: \"tmux_rs::libc::basename\", line: 310 },\n";
        assert_eq!(
            panic_summary(text),
            "panic! \"boom\"\n    at { fn: \"tmux_rs::libc::basename\", line: 310 }"
        );
    }

    #[test]
    fn options() {
        let root = std::path::Path::new("/r");
        let o = Options::parse(&[], root).unwrap();
        assert_eq!(o.bin, PathBuf::from("/r/target/debug/tmux-rs"));
        assert!(o.reduce && !o.all && !o.corpus_only);
        let args: Vec<String> = ["--seed", "9", "--count", "5", "--format", "#{a}", "--all"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let o = Options::parse(&args, root).unwrap();
        assert_eq!((o.seed, o.count, o.all), (9, 5, true));
        assert_eq!(o.formats, vec!["#{a}"]);
        assert!(Options::parse(&["--seed".to_string()], root).is_err());
        assert!(Options::parse(&["--bogus".to_string()], root).is_err());
    }

    #[test]
    fn setup_starts_with_a_session() {
        let s = setup_commands("/h");
        assert_eq!(s[0][0], "new-session");
        assert!(s.iter().all(|c| !c.is_empty()));
    }
}
