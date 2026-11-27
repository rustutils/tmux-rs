//! Tests with a fake environment that matches the scratch 3.8 server of
//! cases/gen.py: session main with windows first, second and third (the
//! current one, second is the last window), and the user options set
//! there. display -p expands with time, so the cases do too.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::rc::Rc;
use std::time::Instant;

use crate::cases38::CASES;
use crate::*;

/// Expansion without time, as format_expand.
const PLAIN: ExpandFlags = ExpandFlags { time: false };
/// Expansion with time, as format_expand_time and display -p.
const TIME: ExpandFlags = ExpandFlags { time: true };

/// One window of the fake session.
struct Window {
    index: u32,
    name: &'static str,
    flags: &'static str,
    last: bool,
    active: bool,
}

const WINDOWS: [Window; 3] = [
    Window {
        index: 0,
        name: "first",
        flags: "",
        last: false,
        active: false,
    },
    Window {
        index: 1,
        name: "second",
        flags: "-",
        last: true,
        active: false,
    },
    Window {
        index: 2,
        name: "third",
        flags: "*",
        last: false,
        active: true,
    },
];

/// The options the cases use: the user options of gen.py and 3.8's
/// defaults for the status line.
const OPTIONS: &[(&str, &str)] = &[
    ("@a", "hello"),
    ("@empty", ""),
    ("@zero", "0"),
    ("@one", "1"),
    ("@num", "42"),
    ("@path", "/usr/local/lib/libfoo.so.1"),
    ("@fmt", "#{@a}-#{@num}"),
    ("@hashv", "x#{@a}"),
    ("@sp", "it's a test"),
    ("@wide", "日本語x"),
    ("@style", "#[fg=red]ab#[default]cd"),
    ("@dollar", "$HOME \"q\""),
    ("@t", "1700000000"),
    ("@tn", "#{?@one,%%%%,x}"),
    ("@pct", "100%%"),
    ("@rec", "#{E:@rec}"),
    ("@hash", "a#b"),
    ("status-right", "R"),
    ("status-left", "[#{session_name}] "),
    ("status-left-length", "10"),
    ("status-left-style", "default"),
    ("status-right-length", "40"),
    ("status-right-style", "default"),
    ("status-justify", "left"),
    ("window-status-activity-style", "reverse"),
    ("window-status-bell-style", "reverse"),
    (
        "window-status-current-format",
        "#I:#W#{?window_flags,#{window_flags}, }",
    ),
    ("window-status-current-style", "underscore"),
    (
        "window-status-format",
        "#I:#W#{?window_flags,#{window_flags}, }",
    ),
    ("window-status-last-style", "default"),
    ("window-status-separator", " "),
    ("window-status-style", "default"),
    ("status-format[0]", STATUS_FORMAT_0),
];

/// 3.8's default status-format[0].
const STATUS_FORMAT_0: &str = "#[align=left range=left #{E:status-left-style}]#[push-default]#{T;=/#{status-left-length}:status-left}#[pop-default]#[norange default]#[list=on align=#{status-justify}]#[list=left-marker]<#[list=right-marker]>#[list=on]#{W:#[range=window|#{window_index} #{E:window-status-style}#{?#{&&:#{window_last_flag},#{!=:#{E:window-status-last-style},default}}, #{E:window-status-last-style},}#{?#{&&:#{window_bell_flag},#{!=:#{E:window-status-bell-style},default}}, #{E:window-status-bell-style},#{?#{&&:#{||:#{window_activity_flag},#{window_silence_flag}},#{!=:#{E:window-status-activity-style},default}}, #{E:window-status-activity-style},}}]#[push-default]#{T:window-status-format}#[pop-default]#[norange default]#{?loop_last_flag,,#{E:window-status-separator}},#[range=window|#{window_index} list=focus #{?#{!=:#{E:window-status-current-style},default},#{E:window-status-current-style},#{E:window-status-style}}#{?#{&&:#{window_last_flag},#{!=:#{E:window-status-last-style},default}}, #{E:window-status-last-style},}#{?#{&&:#{window_bell_flag},#{!=:#{E:window-status-bell-style},default}}, #{E:window-status-bell-style},#{?#{&&:#{||:#{window_activity_flag},#{window_silence_flag}},#{!=:#{E:window-status-activity-style},default}}, #{E:window-status-activity-style},}}]#[push-default]#{T:window-status-current-format}#[pop-default]#[norange list=on default]#{?loop_last_flag,,#{E:window-status-separator}}}#[nolist align=right range=right #{E:status-right-style}]#[push-default]#{T;=/#{status-right-length}:status-right}#[pop-default]#[norange default]";

/// A job the engine asked for: the raw command and what it expanded to.
type JobCall = (Vec<u8>, Vec<u8>);

/// The fake env: a window of the session (or none), the options, the
/// jobs it was asked about and their output.
struct Fake {
    window: Option<usize>,
    options: Rc<HashMap<Vec<u8>, Vec<u8>>>,
    jobs: Rc<RefCell<Vec<JobCall>>>,
    job_output: Option<Vec<u8>>,
    lookups: Rc<RefCell<u32>>,
    start: Instant,
    /// loop_index and loop_last_flag, inside a loop.
    loop_vars: Option<(usize, bool)>,
}

impl Fake {
    fn new() -> Self {
        let options = OPTIONS
            .iter()
            .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
            .collect();
        Fake {
            window: Some(2),
            options: Rc::new(options),
            jobs: Rc::default(),
            job_output: None,
            lookups: Rc::default(),
            start: Instant::now(),
            loop_vars: None,
        }
    }

    fn child(&self, window: usize, index: usize, last: bool) -> Fake {
        Fake {
            window: Some(window),
            options: Rc::clone(&self.options),
            jobs: Rc::clone(&self.jobs),
            job_output: self.job_output.clone(),
            lookups: Rc::clone(&self.lookups),
            start: self.start,
            loop_vars: Some((index, last)),
        }
    }
}

/// The fixed "now" of the fake: 2026-11-27 00:30:00 UTC.
const NOW: i64 = 1_795_739_400;

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A glob match with *, ? and [...], enough for the cases.
fn glob(p: &[u8], t: &[u8]) -> bool {
    match (p.first(), t.first()) {
        (None, None) => true,
        (Some(b'*'), _) => glob(&p[1..], t) || (!t.is_empty() && glob(p, &t[1..])),
        (Some(b'?'), Some(_)) => glob(&p[1..], &t[1..]),
        (Some(a), Some(b)) if a == b => glob(&p[1..], &t[1..]),
        _ => false,
    }
}

impl Host for Fake {
    fn now(&self) -> i64 {
        NOW
    }

    fn timer_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn local_time(&self, t: i64) -> Option<Tm> {
        let days = t.div_euclid(86400);
        let secs = t.rem_euclid(86400);
        let (y, m, d) = civil(days);
        Some(Tm {
            sec: (secs % 60) as i32,
            min: ((secs / 60) % 60) as i32,
            hour: (secs / 3600) as i32,
            mday: d as i32,
            mon: (m - 1) as i32,
            year: (y - 1900) as i32,
            wday: (days + 4).rem_euclid(7) as i32,
            yday: 0,
            isdst: 0,
        })
    }

    fn strftime(&self, fmt: &[u8], tm: &Tm, _max: usize) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < fmt.len() {
            if fmt[i] != b'%' || i + 1 == fmt.len() {
                out.push(fmt[i]);
                i += 1;
                continue;
            }
            let s = match fmt[i + 1] {
                b'%' => "%".to_string(),
                b'Y' => (tm.year + 1900).to_string(),
                b'y' => format!("{:02}", tm.year % 100),
                b'm' => format!("{:02}", tm.mon + 1),
                b'd' => format!("{:02}", tm.mday),
                b'H' => format!("{:02}", tm.hour),
                b'M' => format!("{:02}", tm.min),
                b'S' => format!("{:02}", tm.sec),
                b'b' | b'h' => MONTHS[tm.mon as usize].to_string(),
                b'a' => DAYS[tm.wday as usize].to_string(),
                c => format!("%{}", c as char),
            };
            out.extend_from_slice(s.as_bytes());
            i += 2;
        }
        Some(out)
    }

    fn fnmatch(&self, pattern: &[u8], text: &[u8], casefold: bool) -> bool {
        if casefold {
            glob(&pattern.to_ascii_lowercase(), &text.to_ascii_lowercase())
        } else {
            glob(pattern, text)
        }
    }

    fn regex_match(&self, pattern: &[u8], text: &[u8], icase: bool) -> bool {
        let Ok(p) = std::str::from_utf8(pattern) else {
            return false;
        };
        match regex::bytes::RegexBuilder::new(p)
            .case_insensitive(icase)
            .build()
        {
            Ok(re) => re.is_match(text),
            Err(_) => false,
        }
    }

    fn regex_sub(&self, pattern: &[u8], with: &[u8], text: &[u8], icase: bool) -> Option<Vec<u8>> {
        // glibc's REG_EXTENDED and REG_ICASE.
        let flags = 1 | if icase { 2 } else { 0 };
        let c = |b: &[u8]| CString::new(b.to_vec()).ok();
        tmux_regsub::regsub(&c(pattern)?, &c(with)?, &c(text)?, flags)
    }

    fn colour(&self, _name: &[u8]) -> Option<i32> {
        None
    }

    fn colour_rgb(&self, _colour: i32) -> Option<u32> {
        None
    }

    fn fuzzy_match(&self, _pattern: &[u8], _text: &[u8], _width: u32) -> Option<Vec<bool>> {
        None
    }
}

impl FormatEnv for Fake {
    fn lookup(&self, name: &[u8], _for_time: bool) -> Option<Value> {
        *self.lookups.borrow_mut() += 1;
        if let Some(v) = self.options.get(name) {
            return Some(Value::Bytes(v.clone()));
        }
        match (self.loop_vars, name) {
            (Some((index, _)), b"loop_index") => {
                return Some(Value::Bytes(index.to_string().into_bytes()));
            }
            (Some((_, last)), b"loop_last_flag") => {
                return Some(Value::Bytes(if last {
                    b"1".to_vec()
                } else {
                    b"0".to_vec()
                }));
            }
            _ => {}
        }
        let w = &WINDOWS[self.window?];
        let flag = |b: bool| Value::Bytes(if b { b"1".to_vec() } else { b"0".to_vec() });
        Some(match name {
            b"session_name" => Value::Bytes(b"main".to_vec()),
            b"window_index" => Value::Bytes(w.index.to_string().into_bytes()),
            b"window_name" => Value::Bytes(w.name.as_bytes().to_vec()),
            b"window_flags" => Value::Bytes(w.flags.as_bytes().to_vec()),
            b"window_last_flag" => flag(w.last),
            b"window_active" => flag(w.active),
            b"window_bell_flag" | b"window_activity_flag" | b"window_silence_flag" => flag(false),
            b"pane_index" => Value::Bytes(b"0".to_vec()),
            b"window_activity" => Value::Time(1_700_000_000),
            _ => return None,
        })
    }

    fn children(
        &self,
        kind: LoopKind,
        sort: LoopSort,
        _flags: &[u8],
    ) -> Option<Vec<LoopChild<'_>>> {
        if kind != LoopKind::Windows {
            return None;
        }
        self.window?;
        let mut order: Vec<usize> = (0..WINDOWS.len()).collect();
        if sort.order == SortOrder::Name {
            order.sort_by_key(|&i| WINDOWS[i].name);
        }
        if sort.reversed {
            order.reverse();
        }
        let n = order.len();
        Some(
            order
                .into_iter()
                .enumerate()
                .map(|(at, i)| LoopChild {
                    env: Box::new(self.child(i, at, at + 1 == n)),
                    active: WINDOWS[i].active,
                })
                .collect(),
        )
    }

    fn job(&self, cmd: &[u8], expanded: &[u8]) -> Option<Vec<u8>> {
        self.jobs
            .borrow_mut()
            .push((cmd.to_vec(), expanded.to_vec()));
        self.job_output.clone()
    }

    fn name_exists(&self, kind: NameKind, name: &[u8]) -> Option<bool> {
        match kind {
            NameKind::Session => Some(name == b"main"),
            NameKind::Window => {
                self.window?;
                Some(WINDOWS.iter().any(|w| w.name.as_bytes() == name))
            }
        }
    }
}

fn show(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Every case from 3.8, expanded with time as display -p does.
#[test]
fn cases_against_3_8() {
    let engine = Engine::new();
    let env = Fake::new();
    let mut failed = Vec::new();
    for (fmt, want) in CASES {
        let got = engine.expand(&env, &env, fmt, TIME);
        if got != *want {
            failed.push(format!(
                "{}\n    3.8:   {}\n    spike: {}",
                show(fmt),
                show(want),
                show(&got)
            ));
        }
    }
    eprintln!(
        "{} of {} cases match 3.8",
        CASES.len() - failed.len(),
        CASES.len()
    );
    for f in &failed {
        eprintln!("{f}");
    }
    assert!(failed.is_empty());
}

#[test]
fn names_that_are_not_utf8_survive() {
    let mut env = Fake::new();
    let mut options = (*env.options).clone();
    options.insert(b"@bad\xff".to_vec(), b"v\xfe".to_vec());
    env.options = Rc::new(options);
    let engine = Engine::new();
    assert_eq!(
        engine.expand(&env, &env, b"#{@bad\xff}|\xfd", PLAIN),
        b"v\xfe|\xfd"
    );
    assert_eq!(engine.expand(&env, &env, b"#{n:@bad\xff}", PLAIN), b"2");
}

#[test]
fn jobs_expand_command_and_output() {
    let mut env = Fake::new();
    env.job_output = Some(b"out #{@a}".to_vec());
    let engine = Engine::new();
    assert_eq!(
        engine.expand(&env, &env, b"[#(echo #{@num})]", PLAIN),
        b"[out hello]"
    );
    assert_eq!(
        env.jobs.borrow()[0],
        (b"echo #{@num}".to_vec(), b"echo 42".to_vec())
    );
    // A job inside a job's output does not run.
    env.job_output = Some(b"#(nested)".to_vec());
    env.jobs.borrow_mut().clear();
    assert_eq!(engine.expand(&env, &env, b"#(x)", PLAIN), b"");
    assert_eq!(env.jobs.borrow().len(), 1);
}

#[test]
fn time_mode_formats_job_command_first() {
    let env = Fake::new();
    let engine = Engine::new();
    engine.expand(&env, &env, b"#(date +%H)", TIME);
    assert_eq!(env.jobs.borrow()[0].0, b"date +00");
}

/// Random formats from the characters that matter must not panic.
#[test]
fn malformed_input_does_not_panic() {
    const ALPHABET: &[u8] = b"#{}()[],:;?=/|&!<>abElqsWTRpne1@%-\xff";
    let env = Fake::new();
    let engine = Engine::new();
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    for _ in 0..20_000 {
        let mut fmt = Vec::new();
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let len = (seed % 24) as usize;
        for _ in 0..len {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            fmt.push(ALPHABET[(seed % ALPHABET.len() as u64) as usize]);
        }
        engine.expand(&env, &env, &fmt, PLAIN);
        engine.expand(&env, &env, &fmt, TIME);
    }
}

#[test]
fn time_values() {
    let env = Fake::new();
    let engine = Engine::new();
    assert_eq!(
        engine.expand(&env, &env, b"#{window_activity}", PLAIN),
        b"1700000000"
    );
    assert_eq!(
        engine.expand(&env, &env, b"#{t:window_activity}", PLAIN),
        b"Tue Nov 14 22:13:20 2023"
    );
    assert_eq!(
        engine.expand(&env, &env, b"#{t/r:window_activity}", PLAIN),
        b"1108d2h"
    );
    assert_eq!(
        engine.expand(&env, &env, b"#{t/d:window_activity}", PLAIN),
        b"95739400"
    );
}

/// Without time, a value with %% inside a conditional keeps both. With
/// time (display -p, status lines) every level formats again, so 3.8
/// prints one %. A tree has to parse again after strftime to match.
#[test]
fn time_mode_formats_each_level() {
    let env = Fake::new();
    let engine = Engine::new();
    assert_eq!(engine.expand(&env, &env, b"#{E:@tn}", PLAIN), b"%%%%");
    assert_eq!(engine.expand(&env, &env, b"#{E:@tn}", TIME), b"%");
}

#[test]
fn window_loop_without_session_stops() {
    let mut env = Fake::new();
    env.window = None;
    let engine = Engine::new();
    assert_eq!(engine.expand(&env, &env, b"a#{W:x}b", PLAIN), b"a");
}

#[test]
fn cache_parses_status_line_once() {
    let env = Fake::new();
    let engine = Engine::new();
    let fmt = STATUS_FORMAT_0.as_bytes();
    let first = engine.expand(&env, &env, fmt, TIME);
    let parses = engine.parse_count();
    let second = engine.expand(&env, &env, fmt, TIME);
    assert_eq!(first, second);
    assert_eq!(engine.parse_count(), parses);
}

/// Times status-format[0]. Run with
/// `cargo test --release -p tmux-format-spike -- --ignored --nocapture`.
#[test]
#[ignore = "timing, run by hand"]
fn status_line_cost() {
    let env = Fake::new();
    let fmt = STATUS_FORMAT_0.as_bytes();
    let n = 20_000;

    let engine = Engine::new();
    engine.expand(&env, &env, fmt, TIME);
    *env.lookups.borrow_mut() = 0;
    let start = Instant::now();
    for _ in 0..n {
        std::hint::black_box(engine.expand(&env, &env, fmt, TIME));
    }
    let cached = start.elapsed() / n;
    let lookups = *env.lookups.borrow() / n;

    let start = Instant::now();
    for _ in 0..n / 10 {
        let engine = Engine::new();
        std::hint::black_box(engine.expand(&env, &env, fmt, TIME));
    }
    let cold = start.elapsed() / (n / 10);

    let start = Instant::now();
    for _ in 0..n {
        std::hint::black_box(parse(fmt));
    }
    let parse_only = start.elapsed() / n;

    eprintln!(
        "status-format[0]: {cached:?} cached, {cold:?} with an empty cache, {parse_only:?} to parse the top level, {lookups} lookups"
    );
}

/// status-format[0] with the window loop written out for three windows
/// whose variables are user options, so it runs without a session. The
/// same string is timed on src/format.rs (format::tests::status_proxy_cost).
pub(crate) const STATUS_PROXY: &str = include_str!("../cases/status_proxy.txt");

/// The options the proxy reads, for both engines.
pub(crate) const PROXY_OPTIONS: &[(&str, &str)] = &[
    ("@w0_index", "0"),
    ("@w1_index", "1"),
    ("@w2_index", "2"),
    ("@w0_last_flag", "0"),
    ("@w1_last_flag", "1"),
    ("@w2_last_flag", "0"),
    ("@w0_last", "0"),
    ("@w1_last", "0"),
    ("@w2_last", "1"),
    ("@w0_bell_flag", "0"),
    ("@w1_bell_flag", "0"),
    ("@w2_bell_flag", "0"),
    ("@w0_activity_flag", "0"),
    ("@w1_activity_flag", "0"),
    ("@w2_activity_flag", "0"),
    ("@w0_silence_flag", "0"),
    ("@w1_silence_flag", "0"),
    ("@w2_silence_flag", "0"),
];

#[test]
#[ignore = "timing, run by hand"]
fn status_proxy_cost() {
    let mut env = Fake::new();
    env.window = None;
    let mut options = (*env.options).clone();
    for (k, v) in PROXY_OPTIONS {
        options.insert(k.as_bytes().to_vec(), v.as_bytes().to_vec());
    }
    env.options = Rc::new(options);
    let fmt = STATUS_PROXY.as_bytes();
    let engine = Engine::new();
    let out = engine.expand(&env, &env, fmt, TIME);
    let n = 20_000;
    let start = Instant::now();
    for _ in 0..n {
        std::hint::black_box(engine.expand(&env, &env, fmt, TIME));
    }
    let cached = start.elapsed() / n;
    let start = Instant::now();
    for _ in 0..n / 10 {
        let engine = Engine::new();
        std::hint::black_box(engine.expand(&env, &env, fmt, TIME));
    }
    let cold = start.elapsed() / (n / 10);
    eprintln!(
        "proxy: {cached:?} cached, {cold:?} with an empty cache, {} bytes out",
        out.len()
    );
    eprintln!("{}", show(&out));
}
