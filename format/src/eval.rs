//! Expanding a parsed format (format_expand1 and format_replace).
//!
//! The functions here follow format.c's: [`expand1`] is format_expand1,
//! [`replace`] is format_replace with its modifier switch, [`find`] is
//! format_find, and the loops, the conditional, the operators and the
//! expression are their C namesakes. Where C scans text at run time
//! (format_skip1) it also checks the time limit, so the evaluator checks
//! it at the same places even though the text was scanned when parsed.
//! With logging on (display -v), each step logs the line C's format_log
//! does, through [`FormatEnv::log`].

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::parse::{Format, Mark, Part, Replace, parse, strip};
use crate::text;
use crate::{ClientQuery, FormatEnv, Host, LoopKind, LoopSort, NameKind, SortOrder, Value};

/// How deep expansions may nest (FORMAT_LOOP_LIMIT).
const LOOP_LIMIT: u32 = 100;
/// How long one expansion may take, in milliseconds (FORMAT_TIME_LIMIT).
const TIME_LIMIT: u64 = 100;
/// The largest width for `=` and `p` (FORMAT_MAX_WIDTH).
const MAX_WIDTH: i64 = 10000;
/// The most repeats for `R` (FORMAT_MAX_REPEAT).
const MAX_REPEAT: i64 = 10000;
/// The longest result of `R` (FORMAT_MAX_REPEAT_SIZE).
const MAX_REPEAT_SIZE: usize = 65536;
/// The largest precision for `e` (FORMAT_MAX_PRECISION).
const MAX_PRECISION: i64 = 100;
/// How long each frame of `A` shows, in milliseconds
/// (FORMAT_CYCLE_PERIOD).
const CYCLE_PERIOD: u64 = 100;
/// The buffer C's format_expand1 formats time into.
const EXPAND_TIME_MAX: usize = 8192;
/// The buffer format_find formats `t/f` into.
const TIME_FORMAT_MAX: usize = 512;
/// The buffer format_pretty_time formats into.
const PRETTY_TIME_MAX: usize = 9;
/// The cache is cleared when it holds this many formats.
const CACHE_LIMIT: usize = 4096;

/// The engine: a cache of parsed formats and the entry points. One per
/// server, there is no global state.
#[derive(Default)]
pub struct Engine {
    cache: RefCell<HashMap<Vec<u8>, Rc<Format>>>,
    parses: Cell<u64>,
}

impl Engine {
    /// A new engine with an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `s`, or returns the parsed format from the cache.
    pub fn parse_cached(&self, s: &[u8]) -> Rc<Format> {
        if let Some(f) = self.cache.borrow().get(s) {
            return Rc::clone(f);
        }
        let parsed = Rc::new(parse(s));
        self.parses.set(self.parses.get() + 1);
        let mut cache = self.cache.borrow_mut();
        if cache.len() >= CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(s.to_vec(), Rc::clone(&parsed));
        parsed
    }

    /// How many times a format was parsed because it was not cached.
    pub fn parse_count(&self) -> u64 {
        self.parses.get()
    }

    /// format_expand, or format_expand_time with `flags.time`.
    ///
    /// The start time for the time limit and the frames of `A` is
    /// [`Host::timer_ms`] now. Whether `A` animates
    /// ([`FormatEnv::is_status`]) and whether to log
    /// ([`FormatEnv::logging`]) are asked of `env` once, and hold for the
    /// children of loops too, as C's format trees copy their flags.
    pub fn expand(
        &self,
        host: &dyn Host,
        env: &dyn FormatEnv,
        fmt: &[u8],
        flags: ExpandFlags,
    ) -> Vec<u8> {
        let parsed = self.parse_cached(fmt);
        let depth = Cell::new(0);
        let st = State {
            engine: self,
            host,
            env,
            top: env,
            start_ms: host.timer_ms(),
            depth: &depth,
            time: flags.time,
            nojobs: false,
            status: env.is_status(),
            logging: env.logging(),
        };
        expand1(&st, &parsed)
    }
}

/// How to expand a format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExpandFlags {
    /// FORMAT_EXPAND_TIME: run strftime over each level first
    /// (format_expand_time).
    pub time: bool,
}

/// format_expand_state: what one expansion carries down.
#[derive(Clone, Copy)]
struct State<'a> {
    engine: &'a Engine,
    host: &'a dyn Host,
    /// The current format tree (a loop's child inside a loop).
    env: &'a dyn FormatEnv,
    /// The env the expansion started with, which gets the log.
    top: &'a dyn FormatEnv,
    /// When the top level expansion started (es->start_time).
    start_ms: u64,
    /// es->loop: how deep format_expand1 calls nest. Shared by every
    /// call with the same state, a new one for each copy C makes
    /// (format_copy_state), so a level that returns early without
    /// counting back down leaks as it does in C.
    depth: &'a Cell<u32>,
    /// FORMAT_EXPAND_TIME: strftime each level first.
    time: bool,
    /// FORMAT_EXPAND_NOJOBS and FORMAT_EXPAND_NOCYCLE, which C always
    /// sets together: inside a job's command or output.
    nojobs: bool,
    /// FORMAT_STATUS, where `A` animates.
    status: bool,
    /// format_logging.
    logging: bool,
}

/// Logs a line made of the byte pieces given, only when logging.
macro_rules! flog {
    ($st:expr, $($piece:expr),+ $(,)?) => {
        if $st.logging {
            let mut message: Vec<u8> = Vec::new();
            $(message.extend_from_slice(AsRef::<[u8]>::as_ref(&$piece));)+
            $st.top.log($st.depth.get(), &message);
        }
    };
}

impl State<'_> {
    /// format_check_time: false once the expansion took too long.
    fn check_time(&self) -> bool {
        let t = self.host.timer_ms().saturating_sub(self.start_ms);
        if t < TIME_LIMIT {
            return true;
        }
        flog!(self, b"reached time limit (", t.to_string(), b")");
        false
    }
}

/// format_true.
fn is_true(s: &[u8]) -> bool {
    !s.is_empty() && s != b"0"
}

/// "1" or "0".
fn bool_value(b: bool) -> Vec<u8> {
    if b { b"1".to_vec() } else { b"0".to_vec() }
}

/// format_expand1.
fn expand1(st: &State<'_>, f: &Format) -> Vec<u8> {
    if f.raw.is_empty() || !st.check_time() {
        return Vec::new();
    }
    if st.depth.get() == LOOP_LIMIT {
        flog!(st, b"reached loop limit (", LOOP_LIMIT.to_string(), b")");
        return Vec::new();
    }
    st.depth.set(st.depth.get() + 1);
    flog!(st, b"expanding format: ", f.raw);

    let out = if st.time && f.raw.contains(&b'%') {
        let tm = st.host.local_time(st.host.now()).unwrap_or_default();
        let formatted = match st.host.strftime(&f.raw, &tm, EXPAND_TIME_MAX) {
            Some(s) if !s.is_empty() => s,
            _ => {
                // C returns here without counting the level back down.
                flog!(st, b"format is too long");
                return Vec::new();
            }
        };
        if formatted != f.raw {
            flog!(st, b"after time expanded: ", formatted);
        }
        let parsed = st.engine.parse_cached(&formatted);
        walk(st, &parsed)
    } else {
        walk(st, f)
    };

    flog!(st, b"result is: ", out);
    st.depth.set(st.depth.get() - 1);
    out
}

/// Expands the parts of a format in order, format_expand1's loop.
fn walk(st: &State<'_>, f: &Format) -> Vec<u8> {
    let mut out = Vec::new();
    for part in &f.parts {
        match part {
            Part::Text(t) => out.extend_from_slice(t),
            Part::Mark(Mark::Style(n)) => flog!(st, b"found #*", n.to_string(), b"["),
            Part::Mark(Mark::Escape(c)) => flog!(st, b"found #", [*c]),
            Part::Stop => break,
            Part::Job(raw, cmd) => {
                flog!(st, b"found #(): ", raw);
                if st.nojobs || st.env.jobs_disabled() {
                    flog!(st, b"#() is disabled");
                } else {
                    let v = job(st, raw, cmd);
                    flog!(st, b"#() result: ", v);
                    out.extend(v);
                }
            }
            Part::Replace(r) => {
                // C finds the end with format_skip1, which checks the
                // time first and stops the expansion when it is over.
                if !st.check_time() {
                    break;
                }
                flog!(st, b"found #{}: ", r.key);
                match replace(st, r) {
                    Some(v) => out.extend(v),
                    None => break,
                }
            }
            Part::Alias(ch, r) => {
                flog!(st, b"found #", [*ch], b": ", r.key);
                match replace(st, r) {
                    Some(v) => out.extend(v),
                    None => break,
                }
            }
        }
    }
    out
}

/// format_job_get, without the job store: the env owns that. The
/// command and the output expand without time, jobs and `A`.
fn job(st: &State<'_>, raw: &[u8], cmd: &Format) -> Vec<u8> {
    let depth = Cell::new(st.depth.get());
    let next = State {
        nojobs: true,
        time: false,
        depth: &depth,
        ..*st
    };
    let expanded = expand1(&next, cmd);
    match st.env.job(raw, &expanded) {
        Some(out) => expand1(&next, &st.engine.parse_cached(&out)),
        None => Vec::new(),
    }
}

/// The modifiers of one replacement after their arguments were expanded,
/// as format_replace's switch leaves them: C's modifier flags and the
/// modifiers it keeps a pointer to (here their index).
struct Mods {
    timestring: bool,
    basename: bool,
    dirname: bool,
    quote_shell: bool,
    quote_shell_sq: bool,
    quote_style: bool,
    quote_args: bool,
    literal: bool,
    expand: bool,
    expand_time: bool,
    length: bool,
    width_of: bool,
    character: bool,
    colour: bool,
    colour_fg: bool,
    colour_bg: bool,
    termcap: bool,
    termfeat: bool,
    client_environ: bool,
    cycle: bool,
    cycle_count: i64,
    not: bool,
    not_not: bool,
    repeat: bool,
    pretty: bool,
    relative: bool,
    difference: bool,
    window_name: bool,
    session_name: bool,
    sessions: bool,
    windows: bool,
    panes: bool,
    clients: bool,
    options: bool,
    environ: bool,
    /// The modifier whose argument is loop_flags (the last `O` or `V`
    /// with exactly one argument).
    loop_flags: Option<usize>,
    cmp: Option<usize>,
    search: Option<usize>,
    mexp: Option<usize>,
    bool_op: Option<usize>,
    subs: Vec<usize>,
    limit: i64,
    marker: Option<usize>,
    width: i64,
    time_format: Option<Vec<u8>>,
}

impl Default for Mods {
    fn default() -> Self {
        Mods {
            timestring: false,
            basename: false,
            dirname: false,
            quote_shell: false,
            quote_shell_sq: false,
            quote_style: false,
            quote_args: false,
            literal: false,
            expand: false,
            expand_time: false,
            length: false,
            width_of: false,
            character: false,
            colour: false,
            colour_fg: false,
            colour_bg: false,
            termcap: false,
            termfeat: false,
            client_environ: false,
            cycle: false,
            cycle_count: 1,
            not: false,
            not_not: false,
            repeat: false,
            pretty: false,
            relative: false,
            difference: false,
            window_name: false,
            session_name: false,
            sessions: false,
            windows: false,
            panes: false,
            clients: false,
            options: false,
            environ: false,
            loop_flags: None,
            cmp: None,
            search: None,
            mexp: None,
            bool_op: None,
            subs: Vec::new(),
            limit: 0,
            marker: None,
            width: 0,
            time_format: None,
        }
    }
}

/// The sort the loop modifiers S, W, P and L ask for, from their
/// argument (None without one). O and V leave the sort alone.
fn loop_sort(c: u8, arg: Option<&[u8]>) -> LoopSort {
    let Some(arg) = arg else {
        let order = match c {
            b'S' => SortOrder::Index,
            b'P' => SortOrder::Creation,
            _ => SortOrder::Order,
        };
        return LoopSort {
            order,
            reversed: false,
        };
    };
    let has = |ch: u8| arg.contains(&ch);
    let order = match c {
        b'S' if has(b'i') => SortOrder::Index,
        b'S' if has(b'n') => SortOrder::Name,
        b'S' if has(b't') => SortOrder::Activity,
        b'S' => SortOrder::Index,
        b'P' if has(b'i') => SortOrder::Index,
        b'P' if has(b'z') => SortOrder::Z,
        b'P' => SortOrder::Creation,
        _ if has(b'i') => SortOrder::Order,
        _ if has(b'n') => SortOrder::Name,
        _ if has(b't') => SortOrder::Activity,
        _ => SortOrder::Order,
    };
    LoopSort {
        order,
        reversed: has(b'r'),
    }
}

/// Reads the expanded modifiers as format_replace's switch does.
fn read_mods(r: &Replace, args: &[Vec<Vec<u8>>], sort: &mut LoopSort) -> Mods {
    let mut m = Mods::default();
    for (i, md) in r.mods.iter().enumerate() {
        let a = &args[i];
        let first = a.first().map(Vec::as_slice);
        let has = |c: u8| first.is_some_and(|s| s.contains(&c));
        match md.name.as_slice() {
            b"m" | b"<" | b">" | b"==" | b"!=" | b"<=" | b">=" => m.cmp = Some(i),
            b"!" => m.not = true,
            b"!!" => m.not_not = true,
            b"||" | b"&&" => m.bool_op = Some(i),
            b"C" => m.search = Some(i),
            b"s" if a.len() >= 2 => m.subs.push(i),
            b"=" if !a.is_empty() => {
                m.limit = text::strtonum(&a[0], -MAX_WIDTH, MAX_WIDTH).unwrap_or(0);
                if a.len() >= 2 {
                    m.marker = Some(i);
                }
            }
            b"p" if !a.is_empty() => {
                m.width = text::strtonum(&a[0], -MAX_WIDTH, MAX_WIDTH).unwrap_or(0)
            }
            b"A" => {
                m.cycle = true;
                if let Some(f) = first {
                    m.cycle_count = text::strtonum(f, 1, 100).unwrap_or(1);
                }
            }
            b"w" => m.width_of = true,
            b"e" if (1..=3).contains(&a.len()) => m.mexp = Some(i),
            b"l" => m.literal = true,
            b"a" => m.character = true,
            b"b" => m.basename = true,
            b"c" => {
                m.colour = true;
                m.colour_fg |= has(b'f');
                m.colour_bg |= has(b'b');
            }
            b"d" => m.dirname = true,
            b"n" => m.length = true,
            b"I" => {
                m.termfeat |= has(b'f');
                m.termcap |= has(b'c');
                m.client_environ |= has(b'e');
            }
            b"t" => {
                m.timestring = true;
                if has(b'p') {
                    m.pretty = true;
                } else if has(b'r') {
                    m.relative = true;
                } else if has(b'd') {
                    m.difference = true;
                } else if a.len() >= 2 && has(b'f') {
                    m.time_format = Some(strip(&a[1]));
                }
            }
            b"q" => {
                if a.is_empty() {
                    m.quote_shell = true;
                } else if has(b's') {
                    m.quote_shell_sq = true;
                } else if has(b'e') || has(b'h') {
                    m.quote_style = true;
                } else if has(b'a') {
                    m.quote_args = true;
                }
            }
            b"E" => m.expand = true,
            b"T" => m.expand_time = true,
            b"N" => {
                if a.is_empty() || has(b'w') {
                    m.window_name = true;
                } else if has(b's') {
                    m.session_name = true;
                }
            }
            b"S" | b"W" | b"P" | b"L" => {
                let c = md.name[0];
                match c {
                    b'S' => m.sessions = true,
                    b'W' => m.windows = true,
                    b'P' => m.panes = true,
                    _ => m.clients = true,
                }
                // One static in C: the last of these sets it.
                *sort = loop_sort(c, first);
            }
            b"O" | b"V" => {
                if md.name[0] == b'O' {
                    m.options = true;
                } else {
                    m.environ = true;
                }
                if a.len() == 1 {
                    m.loop_flags = Some(i);
                }
            }
            b"R" => m.repeat = true,
            _ => {}
        }
    }
    m
}

/// format_replace. None when it fails, which stops the expansion.
fn replace(st: &State<'_>, r: &Replace) -> Option<Vec<u8>> {
    // C expands every argument while it builds the list.
    let args: Vec<Vec<Vec<u8>>> = r
        .mods
        .iter()
        .map(|m| m.args.iter().map(|a| expand1(st, a)).collect())
        .collect();

    // C sets the sort to its defaults first.
    let mut sort = LoopSort {
        order: SortOrder::Order,
        reversed: false,
    };
    if st.logging {
        for (i, md) in r.mods.iter().enumerate() {
            flog!(st, b"modifier ", i.to_string(), b" is ", md.name);
            for (j, a) in args[i].iter().enumerate() {
                flog!(
                    st,
                    b"modifier ",
                    i.to_string(),
                    b" argument ",
                    j.to_string(),
                    b": ",
                    a
                );
            }
        }
    }
    let m = read_mods(r, &args, &mut sort);

    let Some(mut value) = value_of(st, r, &m, &args, sort) else {
        flog!(st, b"failed ", r.key);
        return None;
    };

    // Expand again if required.
    if m.expand {
        value = expand1(st, &st.engine.parse_cached(&value));
    } else if m.expand_time {
        let depth = Cell::new(st.depth.get());
        let next = State {
            time: true,
            depth: &depth,
            ..*st
        };
        value = expand1(&next, &st.engine.parse_cached(&value));
    }

    // The substitution's arguments were expanded with the list and are
    // expanded again here, as in C.
    for &i in &m.subs {
        let left = expand1(st, &st.engine.parse_cached(&args[i][0]));
        let right = expand1(st, &st.engine.parse_cached(&args[i][1]));
        let icase = args[i].get(2).is_some_and(|f| f.contains(&b'i'));
        if let Some(new) = st.host.regex_sub(&left, &right, &value, icase) {
            value = new;
        }
        flog!(st, b"substitute '", left, b"' to '", right, b"': ", value);
    }

    // Truncate the value if needed.
    if m.limit != 0 {
        let marker = m.marker.map(|i| args[i][1].as_slice());
        let trimmed = if m.limit > 0 {
            text::trim_left(&value, m.limit as u32)
        } else {
            text::trim_right(&value, (-m.limit) as u32)
        };
        value = match marker {
            Some(mk) if trimmed != value => {
                if m.limit > 0 {
                    [trimmed.as_slice(), mk].concat()
                } else {
                    [mk, trimmed.as_slice()].concat()
                }
            }
            _ => trimmed,
        };
        flog!(
            st,
            b"applied length limit ",
            m.limit.to_string(),
            b": ",
            value
        );
    }

    // Pad the value if needed.
    if m.width != 0 {
        value = if m.width > 0 {
            text::pad(&value, m.width as u32, true)
        } else {
            text::pad(&value, (-m.width) as u32, false)
        };
        flog!(
            st,
            b"applied padding width ",
            m.width.to_string(),
            b": ",
            value
        );
    }

    // Replace with the length or width if needed.
    if m.length {
        value = value.len().to_string().into_bytes();
        flog!(st, b"replacing with length: ", value);
    }
    if m.width_of {
        value = text::format_width(&value).to_string().into_bytes();
        flog!(st, b"replacing with width: ", value);
    }

    flog!(st, b"replaced '", r.key, b"' with '", value, b"'");
    Some(value)
}

/// The value of a replacement before the modifiers that apply to every
/// value (format_replace up to its done label). None fails it.
fn value_of(
    st: &State<'_>,
    r: &Replace,
    m: &Mods,
    args: &[Vec<Vec<u8>>],
    sort: LoopSort,
) -> Option<Vec<u8>> {
    let copy = r.copy.as_slice();

    // Look up client capability, feature or environment.
    if m.termcap || m.termfeat || m.client_environ {
        // C sets the value for each in this order, so the last wins.
        let query = if m.client_environ {
            ClientQuery::Environ
        } else if m.termfeat {
            ClientQuery::Feature
        } else {
            ClientQuery::Capability
        };
        return Some(st.env.client_query(query, copy).unwrap_or_default());
    }

    // Is this an animation cycle?
    if m.cycle {
        let value = cycle(st, copy, m.cycle_count);
        flog!(st, b"cycle '", copy, b"' is: ", value);
        return Some(value);
    }

    // Is this a literal string?
    if m.literal {
        flog!(st, b"literal string is '", copy, b"'");
        return Some(r.literal().to_vec());
    }

    // Is this a character?
    if m.character {
        let new = expand1(st, r.whole());
        return Some(match text::strtonum(&new, 32, 126) {
            Some(c) => vec![c as u8],
            None => Vec::new(),
        });
    }

    // Is this a colour?
    if m.colour {
        let new = expand1(st, r.whole());
        return Some(colour(st, m, &new));
    }

    // Is this a loop, operator, comparison or condition?
    let loop_kind = if m.sessions {
        Some(LoopKind::Sessions)
    } else if m.windows {
        Some(LoopKind::Windows)
    } else if m.panes {
        Some(LoopKind::Panes)
    } else if m.clients {
        Some(LoopKind::Clients)
    } else if m.options {
        Some(LoopKind::Options)
    } else if m.environ {
        Some(LoopKind::Environ)
    } else {
        None
    };
    if let Some(kind) = loop_kind {
        let flags = match kind {
            LoopKind::Options | LoopKind::Environ => {
                m.loop_flags.map_or(&b""[..], |i| args[i][0].as_slice())
            }
            _ => b"",
        };
        return loop_over(st, r, kind, sort, flags);
    }
    if m.window_name {
        // C fails before it expands the name when there is no session.
        if st.env.name_exists(NameKind::Window, b"").is_none() {
            flog!(st, b"window name but no session");
            return None;
        }
        let name = expand1(st, r.whole());
        return Some(bool_value(
            st.env.name_exists(NameKind::Window, &name).unwrap_or(false),
        ));
    }
    if m.session_name {
        let name = expand1(st, r.whole());
        return Some(bool_value(
            st.env
                .name_exists(NameKind::Session, &name)
                .unwrap_or(false),
        ));
    }
    if let Some(i) = m.search {
        let new = expand1(st, r.whole());
        let flags = args[i].first().map(Vec::as_slice).unwrap_or(b"");
        let regex = flags.contains(&b'r');
        let icase = flags.contains(&b'i');
        return Some(match st.env.search_pane(&new, regex, icase) {
            None => {
                flog!(st, b"search '", new, b"' but no pane");
                b"0".to_vec()
            }
            Some(n) => {
                if st.logging {
                    let id = st.env.pane_id().unwrap_or(0);
                    flog!(st, b"search '", new, b"' pane %", id.to_string());
                }
                n.to_string().into_bytes()
            }
        });
    }
    if m.repeat {
        let Some((l, rt)) = choose(st, r) else {
            flog!(st, b"repeat syntax error: ", copy);
            return None;
        };
        let left = expand1(st, l);
        let right = expand1(st, rt);
        return Some(match text::strtonum(&right, 1, MAX_REPEAT) {
            None => Vec::new(),
            Some(n) if !left.is_empty() && n as usize > MAX_REPEAT_SIZE / left.len() => {
                flog!(st, b"repeat is too long: ", copy);
                Vec::new()
            }
            Some(n) => left.repeat(n as usize),
        });
    }
    if m.not || m.not_not {
        // format_bool_op_1. With both, `!` wins.
        let v = expand1(st, r.whole());
        return Some(bool_value(is_true(&v) != m.not));
    }
    if let Some(i) = m.bool_op {
        return Some(bool_op_n(st, r, r.mods[i].name == b"&&"));
    }
    if let Some(i) = m.cmp {
        let name = r.mods[i].name.as_slice();
        let Some((l, rt)) = choose(st, r) else {
            flog!(st, b"compare ", name, b" syntax error: ", copy);
            return None;
        };
        let left = expand1(st, l);
        let right = expand1(st, rt);
        flog!(st, b"compare ", name, b" left is: ", left);
        flog!(st, b"compare ", name, b" right is: ", right);
        let ord = left.cmp(&right);
        return Some(match name {
            b"==" => bool_value(ord.is_eq()),
            b"!=" => bool_value(ord.is_ne()),
            b"<" => bool_value(ord.is_lt()),
            b">" => bool_value(ord.is_gt()),
            b"<=" => bool_value(ord.is_le()),
            b">=" => bool_value(ord.is_ge()),
            _ => matches(st, &args[i], &left, &right),
        });
    }
    if copy.first() == Some(&b'?') {
        return Some(conditional(st, r, m));
    }
    if let Some(i) = m.mexp {
        return Some(expression(st, r, &args[i]).unwrap_or_default());
    }
    if copy.windows(2).any(|w| w == b"#{") {
        flog!(st, b"expanding inner format '", copy, b"'");
        return Some(expand1(st, r.whole()));
    }
    Some(match find(st, copy, m) {
        None => {
            flog!(st, b"format '", copy, b"' not found");
            Vec::new()
        }
        Some(v) => {
            flog!(st, b"format '", copy, b"' found: ", v);
            v
        }
    })
}

/// format_choose without expanding: the text split at its first comma.
/// None without a comma, or when the time is up (format_skip1 checks).
fn choose<'r>(st: &State<'_>, r: &'r Replace) -> Option<(&'r Format, &'r Format)> {
    if !st.check_time() {
        return None;
    }
    r.choose().map(|(a, b)| (a, b))
}

/// The colour modifier `c`: the colour as RRGGBB, or with `f` or `b`
/// the escape sequence that sets it.
fn colour(st: &State<'_>, m: &Mods, new: &[u8]) -> Vec<u8> {
    if m.colour_fg || m.colour_bg {
        if new.eq_ignore_ascii_case(b"none") {
            return b"\x1b[0m".to_vec();
        }
        return match st.host.colour(new) {
            Some(c) => st.env.colour_escape(c, m.colour_bg).unwrap_or_default(),
            None => Vec::new(),
        };
    }
    match st.host.colour(new).and_then(|c| st.host.colour_rgb(c)) {
        Some(rgb) => format!("{:06x}", rgb & 0xff_ffff).into_bytes(),
        None => Vec::new(),
    }
}

/// format_cycle: the frame of `A` for the time the expansion started.
/// The frames are the raw text split at every comma.
fn cycle(st: &State<'_>, frames: &[u8], count: i64) -> Vec<u8> {
    if !st.status || st.nojobs || frames.is_empty() {
        return Vec::new();
    }
    let n = frames.iter().filter(|&&c| c == b',').count() as u64 + 1;
    let index = (st.start_ms / (count as u64 * CYCLE_PERIOD)) % n;
    if n > 1 {
        st.env.start_cycle_timer();
    }
    frames
        .split(|&c| c == b',')
        .nth(index as usize)
        .unwrap_or_default()
        .to_vec()
}

/// format_bool_op_n: `||` and `&&` over the operands split at every
/// comma, stopping as soon as the result is known.
fn bool_op_n(st: &State<'_>, r: &Replace, and: bool) -> Vec<u8> {
    let op: &[u8] = if and { b"&&" } else { b"||" };
    let segs = r.split();
    let mut result = and;
    for (i, (raw, operand)) in segs.iter().enumerate() {
        if result != and {
            break;
        }
        if !st.check_time() {
            // format_skip1 found nothing, so C expands the rest, which
            // gives nothing once the time is up.
            if !(i + 1 == segs.len() && raw.is_empty()) {
                st.check_time();
            }
            flog!(st, b"operator ", op, b" has operand: ");
            result = false;
            break;
        }
        let v = expand1(st, operand);
        flog!(st, b"operator ", op, b" has operand: ", v);
        // result was the identity of the operator, so it is now
        // whatever this operand says.
        result = is_true(&v);
    }
    bool_value(result)
}

/// format_loop_sessions, _windows, _panes, _clients, _options and
/// _environ. S, W and P use the second format for the current item, the
/// others never split.
fn loop_over(
    st: &State<'_>,
    r: &Replace,
    kind: LoopKind,
    sort: LoopSort,
    flags: &[u8],
) -> Option<Vec<u8>> {
    let Some(children) = st.env.children(kind, sort, flags) else {
        match kind {
            LoopKind::Windows => flog!(st, b"window loop but no session"),
            LoopKind::Panes => flog!(st, b"pane loop but no window"),
            _ => {}
        }
        return None;
    };
    let split = matches!(
        kind,
        LoopKind::Sessions | LoopKind::Windows | LoopKind::Panes
    );
    let (all, active) = match split.then(|| choose(st, r)).flatten() {
        Some((a, b)) => (a, Some(b)),
        None => (r.whole(), None),
    };
    let mut out = Vec::new();
    for child in &children {
        if st.logging
            && let Some(message) = child.env.loop_message()
        {
            flog!(st, message);
        }
        let fmt = match active {
            Some(act) if child.active => act,
            _ => all,
        };
        let depth = Cell::new(st.depth.get());
        let cst = State {
            env: &*child.env,
            depth: &depth,
            ..*st
        };
        out.extend(expand1(&cst, fmt));
    }
    Some(out)
}

/// format_match: `m`, with fnmatch, a regular expression or the fuzzy
/// matcher.
fn matches(st: &State<'_>, args: &[Vec<u8>], pattern: &[u8], text: &[u8]) -> Vec<u8> {
    let flags = args.first().map(Vec::as_slice).unwrap_or(b"");
    if flags.contains(&b'p') {
        return match_fuzzy(st, pattern, text, true);
    }
    if flags.contains(&b'z') {
        return match_fuzzy(st, pattern, text, false);
    }
    let icase = flags.contains(&b'i');
    let found = if flags.contains(&b'r') {
        st.host.regex_match(pattern, text, icase)
    } else {
        st.host.fnmatch(pattern, text, icase)
    };
    bool_value(found)
}

/// format_match_fuzzy: with `positions` the matched columns joined by
/// commas ("" without a match), otherwise 1 or 0.
fn match_fuzzy(st: &State<'_>, pattern: &[u8], text: &[u8], positions: bool) -> Vec<u8> {
    let width = text::format_width(text).max(1);
    let Some(bits) = st.host.fuzzy_match(pattern, text, width) else {
        return if positions { Vec::new() } else { b"0".to_vec() };
    };
    if !positions {
        return b"1".to_vec();
    }
    let mut out = Vec::new();
    for i in 0..width as usize {
        if !bits.get(i).copied().unwrap_or(false) {
            continue;
        }
        if !out.is_empty() {
            out.push(b',');
        }
        out.extend_from_slice(i.to_string().as_bytes());
    }
    out
}

/// The conditional: pairs of condition and value, then an optional else.
fn conditional(st: &State<'_>, r: &Replace, m: &Mods) -> Vec<u8> {
    let rest = &r.copy[1..];
    let segs = r.cond();
    let mut i = 0;
    loop {
        // format_skip1 for the end of the condition.
        if i + 1 == segs.len() || !st.check_time() {
            flog!(st, b"no condition matched in '", rest, b"'; using last arg");
            if i + 1 == segs.len() {
                return expand1(st, &segs[i].1);
            }
            return Vec::new();
        }
        let (raw, parsed) = &segs[i];
        flog!(st, b"condition is: ", raw);
        let found = match find(st, raw, m) {
            Some(v) => {
                flog!(st, b"condition '", raw, b"' found: ", v);
                v
            }
            None => {
                // Not a name: expand it, and if that changes nothing it
                // is false.
                let v = expand1(st, parsed);
                if v == *raw {
                    flog!(st, b"condition '", raw, b"' not found; assuming false");
                    Vec::new()
                } else {
                    v
                }
            }
        };
        // format_skip1 for the end of the value.
        let time_ok = st.check_time();
        let more = time_ok && i + 2 < segs.len();
        if is_true(&found) {
            flog!(st, b"condition '", raw, b"' is true");
            if time_ok {
                return expand1(st, &segs[i + 1].1);
            }
            return Vec::new();
        }
        flog!(st, b"condition '", raw, b"' is false");
        if !more {
            flog!(
                st,
                b"no condition matched in '",
                rest,
                b"'; using empty string"
            );
            return Vec::new();
        }
        i += 2;
    }
}

/// format_find: a lookup with the modifiers that apply to it.
fn find(st: &State<'_>, name: &[u8], m: &Mods) -> Option<Vec<u8>> {
    let (found, mut t) = match st.env.lookup(name, m.timestring)? {
        Value::Bytes(b) => (Some(b), 0),
        Value::Time(t) => (None, t),
    };

    if m.timestring {
        if t == 0 {
            t = found
                .and_then(|f| text::strtonum(&f, 0, i64::MAX))
                .unwrap_or(0);
        }
        if t == 0 {
            return None;
        }
        let now = st.host.now();
        if m.relative {
            return text::relative_time(now, t);
        }
        if m.difference {
            return Some(now.wrapping_sub(t).to_string().into_bytes());
        }
        if m.pretty {
            return Some(pretty_time(st, now, t));
        }
        let tm = st.host.local_time(t)?;
        return match &m.time_format {
            Some(f) => Some(
                st.host
                    .strftime(f, &tm, TIME_FORMAT_MAX)
                    .unwrap_or_default(),
            ),
            None => Some(text::ctime(&tm)),
        };
    }

    let mut found = match found {
        Some(f) => f,
        None if t != 0 => t.to_string().into_bytes(),
        None => return None,
    };
    if m.basename {
        found = text::basename(&found);
    }
    if m.dirname {
        found = text::dirname(&found);
    }
    if m.quote_shell {
        found = text::quote_shell(&found);
    }
    if m.quote_shell_sq {
        found = text::quote_shell_single(&found);
    }
    if m.quote_style {
        found = text::quote_style(&found);
    }
    if m.quote_args {
        found = text::args_escape(&found);
    }
    Some(found)
}

/// format_pretty_time without seconds, into C's 9 byte buffer.
fn pretty_time(st: &State<'_>, now: i64, t: i64) -> Vec<u8> {
    let now = now.max(t);
    let age = now - t;
    let (Some(now_tm), Some(tm)) = (st.host.local_time(now), st.host.local_time(t)) else {
        return Vec::new();
    };
    let fmt: &[u8] = if age < 24 * 3600 {
        b"%H:%M"
    } else if (tm.year == now_tm.year && tm.mon == now_tm.mon) || age < 28 * 24 * 3600 {
        b"%a%d"
    } else if (tm.year == now_tm.year && tm.mon < now_tm.mon)
        || (tm.year == now_tm.year - 1 && tm.mon > now_tm.mon)
    {
        b"%d%b"
    } else {
        b"%h%y"
    };
    st.host
        .strftime(fmt, &tm, PRETTY_TIME_MAX)
        .unwrap_or_default()
}

/// format_replace_expression: `e|op|flags|precision:left,right`.
fn expression(st: &State<'_>, r: &Replace, args: &[Vec<u8>]) -> Option<Vec<u8>> {
    #[derive(Clone, Copy)]
    enum Op {
        Add,
        Sub,
        Mul,
        Div,
        Mod,
        Eq,
        Ne,
        Gt,
        Ge,
        Lt,
        Le,
    }
    let op = match args[0].as_slice() {
        b"+" => Op::Add,
        b"-" => Op::Sub,
        b"*" => Op::Mul,
        b"/" => Op::Div,
        b"%" | b"m" => Op::Mod,
        b"==" => Op::Eq,
        b"!=" => Op::Ne,
        b">" => Op::Gt,
        b"<" => Op::Lt,
        b">=" => Op::Ge,
        b"<=" => Op::Le,
        _ => {
            flog!(st, b"expression has no valid operator: '", args[0], b"'");
            return None;
        }
    };

    // The second argument may be flags.
    let mut use_fp = false;
    let mut prec = 0;
    if args.len() >= 2 && args[1].contains(&b'f') {
        use_fp = true;
        prec = 2;
    }

    // The third argument may be precision.
    if args.len() >= 3 {
        match text::strtonum_err(&args[2], -MAX_PRECISION, MAX_PRECISION) {
            Ok(p) => prec = p,
            Err(e) => {
                flog!(st, b"expression precision ", e, b": ", args[2]);
                return None;
            }
        }
    }

    let Some((l, rt)) = choose(st, r) else {
        flog!(st, b"expression syntax error");
        return None;
    };
    let left = expand1(st, l);
    let right = expand1(st, rt);
    let Some(mut ml) = text::strtod(&left) else {
        flog!(st, b"expression left side is invalid: ", left);
        return None;
    };
    let Some(mut mr) = text::strtod(&right) else {
        flog!(st, b"expression right side is invalid: ", right);
        return None;
    };

    if !use_fp {
        ml = text::to_long_long(ml);
        mr = text::to_long_long(mr);
    }
    flog!(
        st,
        b"expression left side is: ",
        text::print_float(prec, ml)
    );
    flog!(
        st,
        b"expression right side is: ",
        text::print_float(prec, mr)
    );

    let b = |v: bool| if v { 1.0 } else { 0.0 };
    let result = match op {
        Op::Add => text::x86_nan(ml, mr, ml + mr),
        Op::Sub => text::x86_nan(ml, mr, ml - mr),
        Op::Mul => text::x86_nan(ml, mr, ml * mr),
        Op::Div => text::x86_nan(ml, mr, ml / mr),
        Op::Mod => text::fmod(ml, mr),
        Op::Eq => b((ml - mr).abs() < 1e-9),
        Op::Ne => b((ml - mr).abs() > 1e-9),
        Op::Gt => b(ml > mr),
        Op::Ge => b(ml >= mr),
        Op::Lt => b(ml < mr),
        Op::Le => b(ml <= mr),
    };
    let value = if use_fp {
        text::print_float(prec, result)
    } else {
        text::print_float(prec, text::to_long_long(result))
    };
    flog!(st, b"expression result is ", value);
    Some(value)
}
