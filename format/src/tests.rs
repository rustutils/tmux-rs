//! Tests with a fake environment that matches the scratch 3.8 server of
//! cases/gen.py: session main with windows first, second and third (the
//! current one, second is the last window, third has two panes), session
//! other with window o1, the user options and environment set there, and
//! the session environment that update-environment leaves. display -p
//! expands with time, so the cases do too.

use std::cell::{Cell, RefCell};
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

/// Options set on one session, window or pane.
type Opts = &'static [(&'static str, &'static str)];

/// One pane of a fake window.
struct Pane {
    index: u32,
    id: u32,
    options: Opts,
}

/// One window of a fake session.
struct Window {
    index: u32,
    id: u32,
    name: &'static str,
    flags: &'static str,
    options: Opts,
    panes: &'static [Pane],
}

/// A fake session. The current window and pane are the first active.
struct Session {
    id: u32,
    name: &'static str,
    windows: &'static [Window],
    current: usize,
    last: Option<usize>,
    options: Opts,
    /// Name, value (None when removed) and whether it is hidden.
    environ: &'static [(&'static str, Option<&'static str>, bool)],
}

/// What update-environment leaves in a session made from gen.py's
/// small environment: every name removed.
macro_rules! removed {
    ($($extra:expr),*) => {
        &[
            ("DISPLAY", None, false),
            $($extra,)*
            ("KRB5CCNAME", None, false),
            ("MSYSTEM", None, false),
            ("SSH_AGENT_PID", None, false),
            ("SSH_ASKPASS", None, false),
            ("SSH_AUTH_SOCK", None, false),
            ("SSH_CONNECTION", None, false),
            ("WAYLAND_DISPLAY", None, false),
            ("WINDOWID", None, false),
            ("XAUTHORITY", None, false),
            ("XDG_CURRENT_DESKTOP", None, false),
            ("XDG_SESSION_DESKTOP", None, false),
            ("XDG_SESSION_TYPE", None, false),
        ]
    };
}

const SESSIONS: [Session; 2] = [
    Session {
        id: 0,
        name: "main",
        windows: &[
            Window {
                index: 0,
                id: 0,
                name: "first",
                flags: "",
                options: &[("automatic-rename", "off")],
                panes: &[Pane {
                    index: 0,
                    id: 0,
                    options: &[],
                }],
            },
            Window {
                index: 1,
                id: 1,
                name: "second",
                flags: "-",
                options: &[("automatic-rename", "off")],
                panes: &[Pane {
                    index: 0,
                    id: 1,
                    options: &[],
                }],
            },
            Window {
                index: 2,
                id: 2,
                name: "third",
                flags: "*",
                options: &[("@wopt", "wv"), ("automatic-rename", "off")],
                panes: &[
                    Pane {
                        index: 0,
                        id: 2,
                        options: &[("@popt", "pv")],
                    },
                    Pane {
                        index: 1,
                        id: 3,
                        options: &[],
                    },
                ],
            },
        ],
        current: 2,
        last: Some(1),
        options: &[("@sopt", "sv"), ("default-size", "80x24")],
        environ: removed!(("FOO", Some("bar"), false), ("HID", Some("secret"), true)),
    },
    Session {
        id: 1,
        name: "other",
        windows: &[Window {
            index: 0,
            id: 3,
            name: "o1",
            flags: "*",
            options: &[("automatic-rename", "off")],
            panes: &[Pane {
                index: 0,
                id: 4,
                options: &[],
            }],
        }],
        current: 0,
        last: None,
        options: &[("default-size", "80x24")],
        environ: removed!(),
    },
];

/// The global options the cases use: the user options of gen.py and
/// 3.8's defaults for the status line.
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
    ("@quote", "a|b&c;d<e>f(g)h{i}j$k`l\\m\"n'o*p?q[r#s t=u%v\tw"),
    ("@dpath", "//a"),
    ("@slash", "a/b//"),
    ("@neg", "-5"),
    ("@float", "2.5"),
    ("@hex", "0x10"),
    ("@red", "red"),
    ("@big", "99999999999999999999"),
    ("@commas", "a,b,c"),
    ("@frames", "x,y"),
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

/// What an attached client answers, by kind (c, f or e) and name.
type ClientData = HashMap<(u8, Vec<u8>), Vec<u8>>;

/// A search the engine asked for: text, regex and icase.
type SearchCall = (Vec<u8>, bool, bool);

/// A log line: depth and message.
type LogLine = (u32, Vec<u8>);

/// The fake env and host: where it is (a session, window and pane, or
/// none), the options, the variables a loop added, and what the tests
/// set up and read back.
#[derive(Clone)]
struct Fake {
    session: Option<usize>,
    window: Option<usize>,
    pane: Option<usize>,
    options: Rc<HashMap<Vec<u8>, Vec<u8>>>,
    /// What a loop added to this child (C's format_add).
    vars: Rc<HashMap<Vec<u8>, Vec<u8>>>,
    jobs: Rc<RefCell<Vec<JobCall>>>,
    job_output: Option<Vec<u8>>,
    lookups: Rc<RefCell<u32>>,
    start: Instant,
    /// A fake clock for timer_ms, which moves on by `step` each call.
    clock: Option<Rc<Cell<u64>>>,
    step: u64,
    /// FORMAT_STATUS.
    status: bool,
    /// How often start_cycle_timer was called.
    cycle_timers: Rc<Cell<u32>>,
    /// The log, when logging.
    log: Option<Rc<RefCell<Vec<LogLine>>>>,
    /// An attached client: its terminal's capabilities and features
    /// and its environment.
    client: Option<Rc<ClientData>>,
    /// What search_pane finds in a pane.
    search: u32,
    /// The search the engine asked for.
    searched: Rc<RefCell<Option<SearchCall>>>,
}

impl Fake {
    fn new() -> Self {
        let options = OPTIONS
            .iter()
            .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
            .collect();
        Fake {
            session: Some(0),
            window: Some(2),
            pane: Some(0),
            options: Rc::new(options),
            vars: Rc::default(),
            jobs: Rc::default(),
            job_output: None,
            lookups: Rc::default(),
            start: Instant::now(),
            clock: None,
            step: 0,
            status: false,
            cycle_timers: Rc::default(),
            log: None,
            client: None,
            search: 0,
            searched: Rc::default(),
        }
    }

    /// No session, window or pane, as a format without a target.
    fn without_session(mut self) -> Self {
        self.session = None;
        self.window = None;
        self.pane = None;
        self
    }

    fn session(&self) -> Option<&'static Session> {
        Some(&SESSIONS[self.session?])
    }

    fn window(&self) -> Option<&'static Window> {
        Some(&self.session()?.windows[self.window?])
    }

    fn pane(&self) -> Option<&'static Pane> {
        Some(&self.window()?.panes[self.pane?])
    }

    /// A child of a loop: this env somewhere else with some variables.
    fn child(
        &self,
        session: Option<usize>,
        window: Option<usize>,
        pane: Option<usize>,
        vars: Vec<(String, String)>,
    ) -> Fake {
        let mut child = self.clone();
        child.session = session;
        child.window = window;
        child.pane = pane;
        child.vars = Rc::new(
            vars.into_iter()
                .map(|(k, v)| (k.into_bytes(), v.into_bytes()))
                .collect(),
        );
        child
    }

    /// The log lines as C prints them for display -v.
    fn log_lines(&self) -> Vec<String> {
        let log = self.log.as_ref().map(|l| l.borrow().clone());
        log.unwrap_or_default()
            .into_iter()
            .map(|(depth, m)| {
                format!(
                    "#{}{}",
                    " ".repeat(depth.min(10) as usize),
                    String::from_utf8_lossy(&m)
                )
            })
            .collect()
    }
}

/// Where a child keeps the line C logs for it. A name with a NUL can
/// not come from a format.
const LOOP_MESSAGE: &[u8] = b"\0loop";

/// loop_index and loop_last_flag.
fn loop_vars(i: usize, n: usize) -> Vec<(String, String)> {
    vec![
        ("loop_index".into(), i.to_string()),
        ("loop_last_flag".into(), u8::from(i + 1 == n).to_string()),
    ]
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

/// colour.h's flags.
const COLOUR_FLAG_256: i32 = 0x0100_0000;
const COLOUR_FLAG_RGB: i32 = 0x0200_0000;

/// colour_256toRGB.
fn colour_256_to_rgb(c: u32) -> u32 {
    const LOW: [u32; 16] = [
        0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xc0c0c0, 0x808080,
        0xff0000, 0x00ff00, 0xffff00, 0x0000ff, 0xff00ff, 0x00ffff, 0xffffff,
    ];
    const LEVELS: [u32; 6] = [0, 95, 135, 175, 215, 255];
    match c {
        0..=15 => LOW[c as usize],
        16..=231 => {
            let c = c - 16;
            LEVELS[(c / 36) as usize] << 16
                | LEVELS[(c / 6 % 6) as usize] << 8
                | LEVELS[(c % 6) as usize]
        }
        _ => {
            let g = 8 + 10 * (c - 232);
            g << 16 | g << 8 | g
        }
    }
}

impl Host for Fake {
    fn now(&self) -> i64 {
        NOW
    }

    fn timer_ms(&self) -> u64 {
        match &self.clock {
            Some(clock) => {
                let t = clock.get();
                clock.set(t + self.step);
                t
            }
            None => self.start.elapsed().as_millis() as u64,
        }
    }

    fn local_time(&self, t: i64) -> Option<Tm> {
        if !(-(1 << 40)..(1 << 40)).contains(&t) {
            return None;
        }
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

    fn strftime(&self, fmt: &[u8], tm: &Tm, max: usize) -> Option<Vec<u8>> {
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
        if out.is_empty() || out.len() >= max {
            return None;
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

    /// colour_fromstring for the names the cases use.
    fn colour(&self, name: &[u8]) -> Option<i32> {
        const NAMES: [&str; 8] = [
            "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
        ];
        let name = String::from_utf8(name.to_ascii_lowercase()).ok()?;
        if let Some(hex) = name.strip_prefix('#') {
            if hex.len() != 6 {
                return None;
            }
            return Some(i32::from_str_radix(hex, 16).ok()? | COLOUR_FLAG_RGB);
        }
        if let Some(n) = name.strip_prefix("colour") {
            let n: i32 = n.parse().ok()?;
            return (0..=255).contains(&n).then_some(n | COLOUR_FLAG_256);
        }
        match name.as_str() {
            "default" => return Some(8),
            "terminal" => return Some(9),
            _ => {}
        }
        if let Some(n) = name.strip_prefix("bright") {
            return NAMES.iter().position(|&x| x == n).map(|i| 90 + i as i32);
        }
        NAMES.iter().position(|&x| x == name).map(|i| i as i32)
    }

    /// colour_force_rgb.
    fn colour_rgb(&self, c: i32) -> Option<u32> {
        if c & COLOUR_FLAG_RGB != 0 {
            return Some((c & 0xff_ffff) as u32);
        }
        if c & COLOUR_FLAG_256 != 0 {
            return Some(colour_256_to_rgb((c & 0xff) as u32));
        }
        match c {
            0..=7 => Some(colour_256_to_rgb(c as u32)),
            90..=97 => Some(colour_256_to_rgb((8 + c - 90) as u32)),
            _ => None,
        }
    }

    /// A leftmost match of the pattern's characters in order, marking
    /// every column of a matched character. The cases only use patterns
    /// whose match is unambiguous, so fzf's choice is the same.
    fn fuzzy_match(&self, pattern: &[u8], text: &[u8], width: u32) -> Option<Vec<bool>> {
        let pattern = std::str::from_utf8(pattern).ok()?;
        let text = std::str::from_utf8(text).ok()?;
        let mut bits = vec![false; width as usize];
        let mut want = pattern.chars().peekable();
        let mut column = 0;
        for c in text.chars() {
            let w = tmux_utf8::char_width(c) as usize;
            if want.peek() == Some(&c) {
                want.next();
                for b in bits.iter_mut().skip(column).take(w) {
                    *b = true;
                }
            }
            column += w;
        }
        want.peek().is_none().then_some(bits)
    }
}

impl FormatEnv for Fake {
    fn lookup(&self, name: &[u8], for_time: bool) -> Option<Value> {
        *self.lookups.borrow_mut() += 1;
        let local = |opts: Opts| {
            opts.iter()
                .find(|(k, _)| k.as_bytes() == name)
                .map(|(_, v)| Value::Bytes(v.as_bytes().to_vec()))
        };
        if let Some(v) = self.options.get(name) {
            return Some(Value::Bytes(v.clone()));
        }
        if let Some(v) = self.pane().and_then(|p| local(p.options)) {
            return Some(v);
        }
        if let Some(v) = self.window().and_then(|w| local(w.options)) {
            return Some(v);
        }
        if let Some(v) = self.session().and_then(|s| local(s.options)) {
            return Some(v);
        }
        let bytes = |s: String| Some(Value::Bytes(s.into_bytes()));
        let flag = |b: bool| bytes(u8::from(b).to_string());
        if let Some(s) = self.session() {
            match name {
                b"session_name" => return bytes(s.name.into()),
                b"session_id" => return bytes(format!("${}", s.id)),
                _ => {}
            }
        }
        if let (Some(s), Some(w), Some(at)) = (self.session(), self.window(), self.window) {
            match name {
                b"window_index" => return bytes(w.index.to_string()),
                b"window_id" => return bytes(format!("@{}", w.id)),
                b"window_name" => return bytes(w.name.into()),
                b"window_flags" => return bytes(w.flags.into()),
                b"window_last_flag" => return flag(s.last == Some(at)),
                b"window_active" => return flag(s.current == at),
                b"window_bell_flag" | b"window_activity_flag" | b"window_silence_flag" => {
                    return flag(false);
                }
                b"window_activity" => return Some(Value::Time(1_700_000_000)),
                _ => {}
            }
        }
        if let (Some(p), Some(at)) = (self.pane(), self.pane) {
            match name {
                b"pane_index" => return bytes(p.index.to_string()),
                b"pane_id" => return bytes(format!("%{}", p.id)),
                b"pane_active" => return flag(at == 0),
                _ => {}
            }
        }
        if let Some(v) = self.vars.get(name) {
            return Some(Value::Bytes(v.clone()));
        }
        if !for_time
            && let Some(s) = self.session()
            && let Some((_, Some(v), _)) = s.environ.iter().find(|(k, _, _)| k.as_bytes() == name)
        {
            return bytes((*v).into());
        }
        None
    }

    fn children(&self, kind: LoopKind, sort: LoopSort, flags: &[u8]) -> Option<Vec<LoopChild<'_>>> {
        let mut out: Vec<(Fake, bool)> = Vec::new();
        match kind {
            LoopKind::Sessions => {
                let mut order: Vec<usize> = (0..SESSIONS.len()).collect();
                if sort.order == SortOrder::Name {
                    order.sort_by_key(|&i| SESSIONS[i].name);
                }
                if sort.reversed {
                    order.reverse();
                }
                let n = order.len();
                for (at, i) in order.into_iter().enumerate() {
                    let s = &SESSIONS[i];
                    // display -p has no client with a session, so no
                    // session is the active one.
                    let mut vars = loop_vars(at, n);
                    vars.push(("\0loop".into(), format!("session loop: ${}", s.id)));
                    let child = self.child(Some(i), Some(s.current), Some(0), vars);
                    out.push((child, false));
                }
            }
            LoopKind::Windows => {
                let s = self.session()?;
                let mut order: Vec<usize> = (0..s.windows.len()).collect();
                if sort.order == SortOrder::Name {
                    order.sort_by_key(|&i| s.windows[i].name);
                }
                if sort.reversed {
                    order.reverse();
                }
                let n = order.len();
                for (at, &i) in order.iter().enumerate() {
                    let mut vars = loop_vars(at, n);
                    let w = &s.windows[i];
                    vars.push((
                        "\0loop".into(),
                        format!("window loop: {} @{}", w.index, w.id),
                    ));
                    let before = at + 1 < n && order[at + 1] == s.current;
                    let after = at > 0 && order[at - 1] == s.current;
                    vars.push(("window_before_active".into(), u8::from(before).to_string()));
                    vars.push(("window_after_active".into(), u8::from(after).to_string()));
                    let mut neighbour = |j: usize, prefix: &str| {
                        let w = &s.windows[j];
                        vars.push((format!("{prefix}_window_index"), w.index.to_string()));
                        vars.push((
                            format!("{prefix}_window_active"),
                            u8::from(j == s.current).to_string(),
                        ));
                        for (k, v) in w.options.iter().filter(|(k, _)| k.starts_with('@')) {
                            vars.push((format!("{prefix}_{k}"), (*v).into()));
                        }
                    };
                    if at + 1 < n {
                        neighbour(order[at + 1], "next");
                    }
                    if at > 0 {
                        neighbour(order[at - 1], "prev");
                    }
                    let child = self.child(self.session, Some(i), Some(0), vars);
                    out.push((child, i == s.current));
                }
            }
            LoopKind::Panes => {
                let w = self.window()?;
                let mut order: Vec<usize> = (0..w.panes.len()).collect();
                if sort.reversed {
                    order.reverse();
                }
                let n = order.len();
                for (at, i) in order.into_iter().enumerate() {
                    let mut vars = loop_vars(at, n);
                    vars.push(("\0loop".into(), format!("pane loop: %{}", w.panes[i].id)));
                    let child = self.child(self.session, self.window, Some(i), vars);
                    out.push((child, i == 0));
                }
            }
            LoopKind::Clients => {}
            LoopKind::Options => {
                let flags = if flags.is_empty() { b"s" } else { flags };
                let opts: Opts = if flags.contains(&b'v') || flags.contains(&b'g') {
                    // Global options: the cases do not loop over them.
                    &[]
                } else if flags.contains(&b'w') {
                    self.window().map_or(&[], |w| w.options)
                } else if flags.contains(&b's') {
                    self.session().map_or(&[], |s| s.options)
                } else if flags.contains(&b'p') {
                    self.pane().map_or(&[], |p| p.options)
                } else {
                    &[]
                };
                let n = opts.len();
                for (at, (k, v)) in opts.iter().enumerate() {
                    let mut vars = loop_vars(at, n);
                    vars.push(("\0loop".into(), format!("option loop: {k}")));
                    for (name, value) in [
                        ("option_name", *k),
                        ("option_value", *v),
                        ("option_is_array", "0"),
                        ("option_array_key", ""),
                        ("option_array_index", ""),
                        ("option_array_first", "0"),
                        ("option_array_last", "0"),
                        ("option_array_count", "0"),
                        ("option_is_hook", "0"),
                        ("option_is_user", if k.starts_with('@') { "1" } else { "0" }),
                    ] {
                        vars.push((name.into(), value.into()));
                    }
                    let child = self.child(self.session, self.window, self.pane, vars);
                    out.push((child, false));
                }
            }
            LoopKind::Environ => {
                let environ: &[(&str, Option<&str>, bool)] = match flags {
                    b"" | b"s" => self.session().map_or(&[], |s| s.environ),
                    _ => &[],
                };
                let n = environ.len();
                for (at, (k, v, hidden)) in environ.iter().enumerate() {
                    let mut vars = loop_vars(at, n);
                    vars.push(("\0loop".into(), format!("environment loop: {k}")));
                    vars.push(("environ_name".into(), (*k).into()));
                    vars.push(("environ_value".into(), v.unwrap_or("").into()));
                    vars.push(("environ_hidden".into(), u8::from(*hidden).to_string()));
                    vars.push(("environ_removed".into(), u8::from(v.is_none()).to_string()));
                    let child = self.child(self.session, self.window, self.pane, vars);
                    out.push((child, false));
                }
            }
        }
        Some(
            out.into_iter()
                .map(|(env, active)| LoopChild {
                    env: Box::new(env),
                    active,
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
            NameKind::Session => Some(SESSIONS.iter().any(|s| s.name.as_bytes() == name)),
            NameKind::Window => Some(
                self.session()?
                    .windows
                    .iter()
                    .any(|w| w.name.as_bytes() == name),
            ),
        }
    }

    fn search_pane(&self, text: &[u8], regex: bool, icase: bool) -> Option<u32> {
        self.pane()?;
        *self.searched.borrow_mut() = Some((text.to_vec(), regex, icase));
        Some(self.search)
    }

    fn client_query(&self, query: ClientQuery, name: &[u8]) -> Option<Vec<u8>> {
        let client = self.client.as_ref()?;
        let key = match query {
            ClientQuery::Capability => b'c',
            ClientQuery::Feature => b'f',
            ClientQuery::Environ => b'e',
        };
        let found = client.get(&(key, name.to_vec())).cloned();
        Some(match query {
            ClientQuery::Environ => found.unwrap_or_default(),
            _ => {
                if found.is_some() {
                    b"1".to_vec()
                } else {
                    b"0".to_vec()
                }
            }
        })
    }

    /// colour_toescape for a terminal with 256 and RGB colours (as C
    /// assumes without an attached client).
    fn colour_escape(&self, c: i32, background: bool) -> Option<Vec<u8>> {
        let o = if background { 40 } else { 30 };
        let s = if c == 8 || c == 9 {
            format!("\x1b[{}m", o + 9)
        } else if c & COLOUR_FLAG_RGB != 0 {
            format!(
                "\x1b[{};2;{};{};{}m",
                o + 8,
                (c >> 16) & 0xff,
                (c >> 8) & 0xff,
                c & 0xff
            )
        } else if c & COLOUR_FLAG_256 != 0 {
            format!("\x1b[{};5;{}m", o + 8, c & 0xff)
        } else if (0..=7).contains(&c) {
            format!("\x1b[{}m", c + o)
        } else if (90..=97).contains(&c) {
            format!("\x1b[{}m", c + o - 30)
        } else {
            return None;
        };
        Some(s.into_bytes())
    }

    fn is_status(&self) -> bool {
        self.status
    }

    fn start_cycle_timer(&self) {
        self.cycle_timers.set(self.cycle_timers.get() + 1);
    }

    fn logging(&self) -> bool {
        self.log.is_some()
    }

    fn log(&self, depth: u32, message: &[u8]) {
        if let Some(log) = &self.log {
            log.borrow_mut().push((depth, message.to_vec()));
        }
    }

    fn loop_message(&self) -> Option<Vec<u8>> {
        self.vars.get(LOOP_MESSAGE).cloned()
    }

    fn pane_id(&self) -> Option<u32> {
        Some(self.pane()?.id)
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
    let env = Fake::new().without_session();
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
/// `cargo test --release -p tmux-format -- --ignored --nocapture`.
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
    let mut env = Fake::new().without_session();
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

impl Fake {
    /// This env with a global option set.
    fn with_option(mut self, name: &str, value: &str) -> Self {
        let mut options = (*self.options).clone();
        options.insert(name.as_bytes().to_vec(), value.as_bytes().to_vec());
        self.options = Rc::new(options);
        self
    }

    /// This env with a clock that starts at `at` and moves on by `step`
    /// milliseconds each time it is read.
    fn with_clock(mut self, at: u64, step: u64) -> Self {
        self.clock = Some(Rc::new(Cell::new(at)));
        self.step = step;
        self
    }

    /// This env with logging on.
    fn logged(mut self) -> Self {
        self.log = Some(Rc::default());
        self
    }
}

/// Expands with the env as host too.
fn ex(env: &Fake, fmt: &str, flags: ExpandFlags) -> String {
    show(&Engine::new().expand(env, env, fmt.as_bytes(), flags))
}

/// What display -v printed in 3.8 for each line of cases/verbose.txt:
/// the log lines, as the adapter prints them, then the result.
#[test]
fn log_matches_display_v() {
    const FORMATS: &str = include_str!("../cases/verbose.txt");
    const EXPECTED: &str = include_str!("../cases/verbose38.txt");
    let blocks: Vec<&str> = EXPECTED.split("----\n").collect();
    for (i, fmt) in FORMATS.lines().filter(|l| !l.is_empty()).enumerate() {
        let env = Fake::new().logged();
        let out = ex(&env, fmt, TIME);
        let mut got = env.log_lines();
        got.push(out);
        let want: Vec<&str> = blocks[i].lines().collect();
        assert_eq!(got, want, "log of {fmt}");
    }
}

/// Without logging nothing reaches the log.
#[test]
fn no_log_without_logging() {
    let env = Fake::new();
    assert_eq!(ex(&env, "#{?@one,#{W:x},b}", TIME), "xxx");
    assert!(env.log_lines().is_empty());
}

/// The loop limit and the time limit log as C does.
#[test]
fn limits_log() {
    let env = Fake::new().logged();
    assert_eq!(ex(&env, "#{E:@rec}x", PLAIN), "x");
    let lines = env.log_lines();
    assert!(lines.contains(&"#          reached loop limit (100)".to_string()));

    let env = Fake::new().logged().with_clock(0, 60);
    assert_eq!(ex(&env, "a#{@a}b", PLAIN), "a");
    assert_eq!(
        env.log_lines(),
        [
            "# expanding format: a#{@a}b",
            "# reached time limit (120)",
            "# result is: a"
        ]
    );
}

/// C checks the time when it looks for the end of a #{}, so a slow
/// expansion stops at the next one. An alias such as #S has no end to
/// look for and still expands.
#[test]
fn time_limit_stops_at_next_replacement() {
    let env = Fake::new().with_clock(0, 60);
    assert_eq!(ex(&env, "a#{@a}b", PLAIN), "a");
    let env = Fake::new().with_clock(0, 60);
    assert_eq!(ex(&env, "a#Sb#{@a}c", PLAIN), "amainb");
    // Past the limit before it starts: nothing at all.
    let env = Fake::new().with_clock(0, 100);
    assert_eq!(ex(&env, "abc", PLAIN), "");
}

/// A long expansion with a real clock ends, whatever it outputs.
#[test]
fn deep_recursion_ends() {
    let env = Fake::new().with_option("@r2", "#{E:@r2}#{E:@r2}");
    assert_eq!(ex(&env, "#{E:@rec}x", TIME), "x");
    // Each level doubles the work, the time limit stops it.
    let start = Instant::now();
    ex(&env, "#{E:@r2}", PLAIN);
    assert!(start.elapsed().as_millis() < 2000);
}

/// A: the frame for the time the expansion started, only on a status
/// line and never inside a job.
#[test]
fn cycle_frames() {
    let at = |t: u64, fmt: &str| {
        let mut env = Fake::new().with_clock(t, 0);
        env.status = true;
        let out = ex(&env, fmt, PLAIN);
        (out, env.cycle_timers.get())
    };
    assert_eq!(at(0, "#{A:a,b,c}"), ("a".into(), 1));
    assert_eq!(at(100, "#{A:a,b,c}"), ("b".into(), 1));
    assert_eq!(at(250, "#{A:a,b,c}"), ("c".into(), 1));
    assert_eq!(at(300, "#{A:a,b,c}"), ("a".into(), 1));
    assert_eq!(at(250, "#{A/2:a,b}"), ("b".into(), 1));
    assert_eq!(at(450, "#{A/2:a,b}"), ("a".into(), 1));
    // A count out of range is 1.
    assert_eq!(at(150, "#{A/101:a,b}"), ("b".into(), 1));
    assert_eq!(at(150, "#{A/0:a,b}"), ("b".into(), 1));
    // One frame never changes, so no timer.
    assert_eq!(at(150, "#{A:a}"), ("a".into(), 0));
    assert_eq!(at(150, "#{A:}x"), ("x".into(), 0));
    // The frames are raw text, split at every comma.
    assert_eq!(at(0, "#{A:#{@a},b}"), ("#{@a}".into(), 1));
    assert_eq!(at(100, "#{A:#{l:x,y},b}"), ("y}".into(), 1));
    assert_eq!(at(0, "#{E;A:#{@a},b}"), ("hello".into(), 1));
    // A comes before l, so l does nothing here.
    assert_eq!(at(0, "#{l;A:a#,b,c}"), ("a#".into(), 1));

    // Not on a status line.
    let env = Fake::new().with_clock(0, 0);
    assert_eq!(ex(&env, "#{A:a,b}x", PLAIN), "x");
    assert_eq!(env.cycle_timers.get(), 0);

    // Not in a job's command or output.
    let mut env = Fake::new().with_clock(0, 0);
    env.status = true;
    env.job_output = Some(b"#{A:a,b}".to_vec());
    assert_eq!(ex(&env, "[#(x #{A:c,d})]", PLAIN), "[]");
    assert_eq!(env.jobs.borrow()[0].1, b"x ");
    assert_eq!(env.cycle_timers.get(), 0);
}

/// I asks the attached client with the raw text after the modifiers.
#[test]
fn client_queries() {
    let env = Fake::new();
    assert_eq!(ex(&env, "#{I/c:colors}x", PLAIN), "x");
    let mut client = HashMap::new();
    client.insert((b'c', b"colors".to_vec()), Vec::new());
    client.insert((b'f', b"RGB".to_vec()), Vec::new());
    client.insert((b'e', b"HOME".to_vec()), b"/home/x".to_vec());
    client.insert((b'e', b"#{@a}".to_vec()), b"raw".to_vec());
    let mut env = Fake::new();
    env.client = Some(Rc::new(client));
    assert_eq!(ex(&env, "#{I/c:colors}", PLAIN), "1");
    assert_eq!(ex(&env, "#{I/c:nosuch}", PLAIN), "0");
    assert_eq!(ex(&env, "#{I/f:RGB}", PLAIN), "1");
    assert_eq!(ex(&env, "#{I/f:sixel}", PLAIN), "0");
    assert_eq!(ex(&env, "#{I/e:HOME}", PLAIN), "/home/x");
    assert_eq!(ex(&env, "#{I/e:NOSUCH}x", PLAIN), "x");
    // The text is not expanded.
    assert_eq!(ex(&env, "#{I/e:#{@a}}", PLAIN), "raw");
    // C sets the value for c, then f, then e, so the last one wins.
    assert_eq!(ex(&env, "#{I/cf:RGB}", PLAIN), "1");
    assert_eq!(ex(&env, "#{I/ce:HOME}", PLAIN), "/home/x");
    assert_eq!(ex(&env, "#{I/c;I/e:HOME}", PLAIN), "/home/x");
    // Without f, c or e it is not a modifier.
    assert_eq!(ex(&env, "#{I/x:@a}", PLAIN), "hello");
    // The modifiers after the value still apply.
    assert_eq!(ex(&env, "#{I/e;n:HOME}", PLAIN), "7");
}

/// C: the line the pane search found, with its flags.
#[test]
fn pane_search() {
    let mut env = Fake::new();
    env.search = 7;
    assert_eq!(ex(&env, "#{C/ri:#{@a}}", PLAIN), "7");
    assert_eq!(
        *env.searched.borrow(),
        Some((b"hello".to_vec(), true, true))
    );
    assert_eq!(ex(&env, "#{C:x}", PLAIN), "7");
    assert_eq!(*env.searched.borrow(), Some((b"x".to_vec(), false, false)));
    let env = Fake::new().without_session();
    assert_eq!(ex(&env, "#{C:x}", PLAIN), "0");
}

/// N without a session fails the replacement before it expands the name.
#[test]
fn window_name_without_session_fails() {
    let env = Fake::new().without_session().logged();
    assert_eq!(ex(&env, "a#{N:#{@a}}b", PLAIN), "a");
    let lines = env.log_lines();
    assert!(lines.contains(&"# window name but no session".to_string()));
    assert!(lines.contains(&"# failed N:#{@a}".to_string()));
    assert!(!lines.iter().any(|l| l.contains("expanding format: #{@a}")));
    assert_eq!(ex(&env, "#{N/s:main}", PLAIN), "1");
}

/// t/r, t/d and t/p against the fake's fixed now.
#[test]
fn relative_times() {
    let at = |ago: i64, fmt: &str| {
        let env = Fake::new().with_option("@x", &(NOW - ago).to_string());
        ex(&env, fmt, PLAIN)
    };
    assert_eq!(at(0, "#{t/r:@x}"), "0s");
    assert_eq!(at(59, "#{t/r:@x}"), "59s");
    assert_eq!(at(60, "#{t/r:@x}"), "1m");
    assert_eq!(at(61, "#{t/r:@x}"), "1m1s");
    assert_eq!(at(3600, "#{t/r:@x}"), "1h");
    assert_eq!(at(3660, "#{t/r:@x}"), "1h1m");
    assert_eq!(at(3661, "#{t/r:@x}"), "1h1m");
    assert_eq!(at(86400, "#{t/r:@x}"), "1d");
    assert_eq!(at(90000, "#{t/r:@x}"), "1d1h");
    // In the future the value is not found.
    assert_eq!(at(-5, "#{t/r:@x}x"), "x");
    assert_eq!(at(-5, "#{?#{t/r:@x},y,n}"), "n");
    assert_eq!(at(-5, "#{t/d:@x}"), "-5");
    assert_eq!(at(100, "#{t/d:@x}"), "100");
    // t/p: today, this month, the last year, older.
    assert_eq!(at(3600, "#{t/p:@x}"), "23:30");
    assert_eq!(at(-3600, "#{t/p:@x}"), "01:30");
    assert_eq!(at(10 * 86400, "#{t/p:@x}"), "Tue17");
    assert_eq!(at(60 * 86400, "#{t/p:@x}"), "28Sep");
    assert_eq!(at(400 * 86400, "#{t/p:@x}"), "Oct25");
    // p wins over r, r over d.
    assert_eq!(at(60, "#{t/rp:@x}"), "00:29");
    assert_eq!(at(60, "#{t/dr:@x}"), "1m");
    // With several t, the flags add up: r is checked first.
    assert_eq!(at(60, "#{t/d;t/r:@x}"), "1m");
    // Zero and times local_time cannot convert are not found.
    assert_eq!(at(NOW, "#{t:@x}x"), "x");
    let env = Fake::new().with_option("@x", "9223372036854775807");
    assert_eq!(ex(&env, "#{t:@x}x", PLAIN), "x");
    assert_eq!(ex(&env, "#{t/f/%%Y:@x}x", PLAIN), "x");
}

/// t/f formats into 512 bytes, and what does not fit is empty.
#[test]
fn time_buffers() {
    let env = Fake::new().with_option("@f", &"%Y".repeat(127));
    assert_eq!(ex(&env, "#{t/f/#{@f}:@t}", PLAIN).len(), 508);
    let env = Fake::new().with_option("@f", &"%Y".repeat(128));
    assert_eq!(ex(&env, "#{t/f/#{@f}:@t}x", PLAIN), "x");
}

/// e converts to long long as x86_64 does: out of range values and NaN
/// give INT64_MIN on every machine.
#[test]
fn expression_cast_is_x86_64() {
    let env = Fake::new();
    let min = "-9223372036854775808";
    assert_eq!(ex(&env, "#{e|/|:1,0}", PLAIN), min);
    assert_eq!(ex(&env, "#{e|/|:-1,0}", PLAIN), min);
    assert_eq!(ex(&env, "#{e|/|:0,0}", PLAIN), min);
    assert_eq!(ex(&env, "#{e|%|:5,0}", PLAIN), min);
    assert_eq!(ex(&env, "#{e|+|:inf,0}", PLAIN), min);
    assert_eq!(ex(&env, "#{e|+|:nan,0}", PLAIN), min);
    assert_eq!(ex(&env, "#{e|*|:1e300,1e300}", PLAIN), min);
    assert_eq!(ex(&env, "#{e|*|:1e18,10}", PLAIN), min);
    assert_eq!(ex(&env, "#{e|+|:9223372036854775807,0}", PLAIN), min);
    assert_eq!(ex(&env, "#{e|-|:-9223372036854775808,0}", PLAIN), min);
    assert_eq!(ex(&env, "#{e|*|:-1,0}", PLAIN), "0");
    // With f there is no cast.
    assert_eq!(ex(&env, "#{e|*|f|0:1e300,1e300}", PLAIN), "inf");
    assert_eq!(ex(&env, "#{e|/|f:0,0}", PLAIN), "-nan");
    assert_eq!(ex(&env, "#{e|*|f:-1,0}", PLAIN), "-0.00");
    // Precision: -100 to 100, negative prints 6 digits, others fail.
    assert_eq!(ex(&env, "#{e|+|f|-1:1,0}", PLAIN), "1.000000");
    assert_eq!(ex(&env, "#{e|+||-100:1,0}", PLAIN), "1.000000");
    assert_eq!(ex(&env, "#{e|+|f|101:1,0}x", PLAIN), "x");
    assert_eq!(ex(&env, "#{e|+|f|100:1,0}", PLAIN).len(), 102);
}

/// Rust's float formatting rounds as glibc's printf does.
#[test]
fn float_printing() {
    let env = Fake::new();
    for (fmt, want) in [
        ("#{e|+|f|1:0.25,0}", "0.2"),
        ("#{e|+|f|1:0.35,0}", "0.3"),
        ("#{e|+|f|0:0.5,0}", "0"),
        ("#{e|+|f|0:1.5,0}", "2"),
        ("#{e|+|f|0:2.5,0}", "2"),
        ("#{e|+|f|2:0.125,0}", "0.12"),
        ("#{e|+|f|0:1e22,0}", "10000000000000000000000"),
        ("#{e|+|f|3:1e-400,0}", "0.000"),
    ] {
        assert_eq!(ex(&env, fmt, PLAIN), want, "{fmt}");
    }
}

/// c: RRGGBB, or the escape for f and b, with the env's terminal.
#[test]
fn colours() {
    let env = Fake::new();
    assert_eq!(ex(&env, "#{c:colour231}", PLAIN), "ffffff");
    assert_eq!(ex(&env, "#{c:colour232}", PLAIN), "080808");
    assert_eq!(ex(&env, "#{c/b:colour2}", PLAIN), "\x1b[48;5;2m");
    assert_eq!(ex(&env, "#{c/f;n:red}", PLAIN), "5");
}

/// Fuzzy matching passes the display width and reads columns.
#[test]
fn fuzzy() {
    let env = Fake::new();
    assert_eq!(ex(&env, "#{m/p:ac,abc}", PLAIN), "0,2");
    assert_eq!(ex(&env, "#{m/z:ac,abc}", PLAIN), "1");
    assert_eq!(ex(&env, "#{m/z:ca,abc}", PLAIN), "0");
}
