//! Expanding a parsed format (format_expand1 and format_replace).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::parse::{Format, Part, Replace, parse, strip};
use crate::text;
use crate::{FormatEnv, Host, LoopKind, LoopSort, NameKind, SortOrder, Value};

/// How deep expansions may nest (FORMAT_LOOP_LIMIT).
const LOOP_LIMIT: u32 = 100;
/// How long one expansion may take, in milliseconds (FORMAT_TIME_LIMIT).
const TIME_LIMIT: u64 = 100;
/// The largest width for `=` and `p` (FORMAT_MAX_WIDTH).
const MAX_WIDTH: i64 = 10000;
/// The most repeats for `R` and the longest result (FORMAT_MAX_REPEAT and
/// FORMAT_MAX_REPEAT_SIZE).
const MAX_REPEAT: i64 = 10000;
const MAX_REPEAT_SIZE: usize = 65536;
/// The largest precision for `e` (FORMAT_MAX_PRECISION).
const MAX_PRECISION: i64 = 100;
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
    pub fn expand(
        &self,
        host: &dyn Host,
        env: &dyn FormatEnv,
        fmt: &[u8],
        flags: ExpandFlags,
    ) -> Vec<u8> {
        let parsed = self.parse_cached(fmt);
        let st = State {
            engine: self,
            host,
            env,
            start_ms: host.timer_ms(),
            depth: 0,
            time: flags.time,
            nojobs: false,
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
    env: &'a dyn FormatEnv,
    start_ms: u64,
    /// How deep format_expand1 calls nest.
    depth: u32,
    /// FORMAT_EXPAND_TIME: strftime each level first.
    time: bool,
    /// FORMAT_EXPAND_NOJOBS: inside a job's command or output.
    nojobs: bool,
}

impl State<'_> {
    /// format_check_time: false once the expansion took too long.
    fn in_time(&self) -> bool {
        self.host.timer_ms().saturating_sub(self.start_ms) < TIME_LIMIT
    }
}

/// format_true.
fn is_true(s: &[u8]) -> bool {
    !s.is_empty() && s != b"0"
}

fn bool_value(b: bool) -> Vec<u8> {
    if b { b"1".to_vec() } else { b"0".to_vec() }
}

/// format_expand1.
fn expand1(st: &State<'_>, f: &Format) -> Vec<u8> {
    if f.raw.is_empty() || !st.in_time() {
        return Vec::new();
    }
    if st.depth == LOOP_LIMIT {
        return Vec::new();
    }
    let inner = State {
        depth: st.depth + 1,
        ..*st
    };
    if st.time && f.raw.contains(&b'%') {
        let Some(tm) = st.host.local_time(st.host.now()) else {
            return Vec::new();
        };
        let formatted = match st.host.strftime(&f.raw, &tm, 8192) {
            Some(s) if !s.is_empty() => s,
            _ => return Vec::new(),
        };
        let parsed = st.engine.parse_cached(&formatted);
        return walk(&inner, &parsed);
    }
    walk(&inner, f)
}

/// Expands the parts of a format in order.
fn walk(st: &State<'_>, f: &Format) -> Vec<u8> {
    let mut out = Vec::new();
    for part in &f.parts {
        match part {
            Part::Text(t) => out.extend_from_slice(t),
            Part::Stop => break,
            Part::Job(raw, cmd) => out.extend(job(st, raw, cmd)),
            Part::Replace(r) => match replace(st, r) {
                Some(v) => out.extend(v),
                None => break,
            },
        }
    }
    out
}

/// format_job_get, without the job store: the env owns that.
fn job(st: &State<'_>, raw: &[u8], cmd: &Format) -> Vec<u8> {
    if st.nojobs || st.env.jobs_disabled() {
        return Vec::new();
    }
    let next = State {
        nojobs: true,
        time: false,
        ..*st
    };
    let expanded = expand1(&next, cmd);
    match st.env.job(raw, &expanded) {
        Some(out) => expand1(&next, &st.engine.parse_cached(&out)),
        None => Vec::new(),
    }
}

/// The modifiers of one replacement after their arguments were expanded,
/// as format_replace's switch leaves them.
#[derive(Default)]
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
    client_query: bool,
    cycle: bool,
    not: bool,
    not_not: bool,
    repeat: bool,
    pretty: bool,
    relative: bool,
    difference: bool,
    window_name: bool,
    session_name: bool,
    loop_kind: Option<LoopKind>,
    options_or_environ: bool,
    /// The modifier index of the comparison, search, expression and
    /// n-ary operator, and of each substitution.
    cmp: Option<usize>,
    search: Option<usize>,
    mexp: Option<usize>,
    bool_op: Option<usize>,
    subs: Vec<usize>,
    limit: i64,
    marker: Option<usize>,
    pad: i64,
    time_format: Option<Vec<u8>>,
}

/// The sort a loop modifier asks for (format_replace's S, W, P and L).
fn loop_sort(c: u8, arg: Option<&[u8]>) -> LoopSort {
    let has = |ch: u8| arg.is_some_and(|a| a.contains(&ch));
    let order = match c {
        b'S' if arg.is_none() || has(b'i') => SortOrder::Index,
        b'S' if has(b'n') => SortOrder::Name,
        b'S' if has(b't') => SortOrder::Activity,
        b'S' => SortOrder::Index,
        b'P' if has(b'i') => SortOrder::Index,
        b'P' if has(b'z') => SortOrder::Z,
        b'P' => SortOrder::Creation,
        _ if arg.is_none() || has(b'i') => SortOrder::Order,
        _ if has(b'n') => SortOrder::Name,
        _ if has(b't') => SortOrder::Activity,
        _ => SortOrder::Order,
    };
    LoopSort {
        order,
        reversed: has(b'r'),
    }
}

/// format_replace. None when it fails, which stops the expansion.
fn replace(st: &State<'_>, r: &Replace) -> Option<Vec<u8>> {
    // C expands every argument while it builds the list.
    let args: Vec<Vec<Vec<u8>>> = r
        .mods
        .iter()
        .map(|m| m.args.iter().map(|a| expand1(st, a)).collect())
        .collect();

    let mut m = Mods::default();
    let mut sort = LoopSort {
        order: SortOrder::Order,
        reversed: false,
    };
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
                m.pad = text::strtonum(&a[0], -MAX_WIDTH, MAX_WIDTH).unwrap_or(0)
            }
            b"A" => m.cycle = true,
            b"w" => m.width_of = true,
            b"e" if (1..=3).contains(&a.len()) => m.mexp = Some(i),
            b"l" => m.literal = true,
            b"a" => m.character = true,
            b"b" => m.basename = true,
            b"c" => m.colour = true,
            b"d" => m.dirname = true,
            b"n" => m.length = true,
            b"I" if has(b'f') || has(b'c') || has(b'e') => m.client_query = true,
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
                // The first loop modifier in this order wins, the last
                // one sets the sort (one static in C).
                let kind = match c {
                    b'S' => LoopKind::Sessions,
                    b'W' => LoopKind::Windows,
                    b'P' => LoopKind::Panes,
                    _ => LoopKind::Clients,
                };
                m.loop_kind = Some(match (m.loop_kind, kind) {
                    (Some(old), new) => loop_priority(old, new),
                    (None, new) => new,
                });
                sort = loop_sort(c, first);
            }
            b"O" | b"V" => m.options_or_environ = true,
            b"R" => m.repeat = true,
            _ => {}
        }
    }

    let value = if m.client_query || m.cycle || m.colour {
        // Need a client and its terminal, or the status line timer. Not
        // in the spike: C gives "" without an attached client.
        Vec::new()
    } else if m.literal {
        r.literal().to_vec()
    } else if m.character {
        let v = expand1(st, r.whole());
        match text::strtonum(&v, 32, 126) {
            Some(c) => vec![c as u8],
            None => Vec::new(),
        }
    } else if let Some(kind) = m.loop_kind {
        // O and V are tested after S, W, P and L in C.
        loop_over(st, r, kind, sort)?
    } else if m.options_or_environ {
        // O: and V: walk option and environment lists. Not in the spike.
        Vec::new()
    } else if m.window_name || m.session_name {
        let name = expand1(st, r.whole());
        let kind = if m.window_name {
            NameKind::Window
        } else {
            NameKind::Session
        };
        bool_value(st.env.name_exists(kind, &name)?)
    } else if let Some(i) = m.search {
        let s = expand1(st, r.whole());
        let flags = args[i].first().map(Vec::as_slice).unwrap_or(b"");
        match st
            .env
            .search_pane(&s, flags.contains(&b'r'), flags.contains(&b'i'))
        {
            Some(n) => n.to_string().into_bytes(),
            None => b"0".to_vec(),
        }
    } else if m.repeat {
        let (l, rt) = r.choose()?;
        let left = expand1(st, l);
        let right = expand1(st, rt);
        match text::strtonum(&right, 1, MAX_REPEAT) {
            None => Vec::new(),
            Some(n) if !left.is_empty() && n as usize > MAX_REPEAT_SIZE / left.len() => Vec::new(),
            Some(n) => left.repeat(n as usize),
        }
    } else if m.not || m.not_not {
        let v = expand1(st, r.whole());
        bool_value(is_true(&v) != m.not)
    } else if let Some(i) = m.bool_op {
        let and = r.mods[i].name == b"&&";
        let mut result = and;
        for (_, operand) in r.split() {
            if result != and {
                break;
            }
            let v = expand1(st, operand);
            // result was the identity of the operator, so it is now
            // whatever this operand says.
            result = is_true(&v);
        }
        bool_value(result)
    } else if let Some(i) = m.cmp {
        let (l, rt) = r.choose()?;
        let left = expand1(st, l);
        let right = expand1(st, rt);
        let ord = left.cmp(&right);
        match r.mods[i].name.as_slice() {
            b"==" => bool_value(ord.is_eq()),
            b"!=" => bool_value(ord.is_ne()),
            b"<" => bool_value(ord.is_lt()),
            b">" => bool_value(ord.is_gt()),
            b"<=" => bool_value(ord.is_le()),
            b">=" => bool_value(ord.is_ge()),
            _ => matches(st, &args[i], &left, &right),
        }
    } else if r.copy.first() == Some(&b'?') {
        conditional(st, r, &m)
    } else if let Some(i) = m.mexp {
        expression(st, r, &args[i]).unwrap_or_default()
    } else if r.copy.windows(2).any(|w| w == b"#{") {
        expand1(st, r.whole())
    } else {
        find(st, &r.copy, &m).unwrap_or_default()
    };

    let mut value = value;
    if m.expand {
        value = expand1(st, &st.engine.parse_cached(&value));
    } else if m.expand_time {
        let next = State { time: true, ..*st };
        value = expand1(&next, &st.engine.parse_cached(&value));
    }

    // The substitution's arguments were expanded with the list and are
    // expanded again here, as in C.
    for &i in &m.subs {
        let pattern = expand1(st, &st.engine.parse_cached(&args[i][0]));
        let with = expand1(st, &st.engine.parse_cached(&args[i][1]));
        let icase = args[i].get(2).is_some_and(|f| f.contains(&b'i'));
        if let Some(new) = st.host.regex_sub(&pattern, &with, &value, icase) {
            value = new;
        }
    }

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
    }
    if m.pad > 0 {
        value = text::pad(&value, m.pad as u32, true);
    } else if m.pad < 0 {
        value = text::pad(&value, (-m.pad) as u32, false);
    }
    if m.length {
        value = value.len().to_string().into_bytes();
    }
    if m.width_of {
        value = text::format_width(&value).to_string().into_bytes();
    }
    Some(value)
}

/// C tests the loop flags in the order S, W, P, L, so with several the
/// first in that order is used.
fn loop_priority(a: LoopKind, b: LoopKind) -> LoopKind {
    let rank = |k| match k {
        LoopKind::Sessions => 0,
        LoopKind::Windows => 1,
        LoopKind::Panes => 2,
        LoopKind::Clients => 3,
        LoopKind::Options => 4,
        LoopKind::Environ => 5,
    };
    if rank(a) <= rank(b) { a } else { b }
}

/// format_loop_sessions, _windows, _panes and _clients.
fn loop_over(st: &State<'_>, r: &Replace, kind: LoopKind, sort: LoopSort) -> Option<Vec<u8>> {
    let (all, active) = match r.choose() {
        Some((a, b)) => (a, Some(b)),
        None => (r.whole(), None),
    };
    let children = st.env.children(kind, sort, b"")?;
    let mut out = Vec::new();
    for child in &children {
        let fmt = match active {
            Some(act) if child.active => act,
            _ => all,
        };
        let cst = State {
            env: &*child.env,
            ..*st
        };
        out.extend(expand1(&cst, fmt));
    }
    Some(out)
}

/// format_match: `m`, with fnmatch or a regular expression.
fn matches(st: &State<'_>, args: &[Vec<u8>], pattern: &[u8], text: &[u8]) -> Vec<u8> {
    let flags = args.first().map(Vec::as_slice).unwrap_or(b"");
    if flags.contains(&b'p') || flags.contains(&b'z') {
        // The fuzzy matcher (fuzzy.rs) would come in here.
        return Vec::new();
    }
    let icase = flags.contains(&b'i');
    let found = if flags.contains(&b'r') {
        st.host.regex_match(pattern, text, icase)
    } else {
        st.host.fnmatch(pattern, text, icase)
    };
    bool_value(found)
}

/// The conditional: pairs of condition and value, then an optional else.
fn conditional(st: &State<'_>, r: &Replace, m: &Mods) -> Vec<u8> {
    let segs = r.cond();
    let mut i = 0;
    loop {
        if i + 1 == segs.len() {
            return expand1(st, &segs[i].1);
        }
        let (raw, parsed) = &segs[i];
        let found = match find(st, raw, m) {
            Some(v) => v,
            None => {
                // Not a name: expand it, and if that changes nothing it
                // is false.
                let v = expand1(st, parsed);
                if v == *raw { Vec::new() } else { v }
            }
        };
        if is_true(&found) {
            return expand1(st, &segs[i + 1].1);
        }
        if i + 2 == segs.len() {
            return Vec::new();
        }
        i += 2;
    }
}

/// format_find: a lookup with the modifiers that apply to it.
fn find(st: &State<'_>, name: &[u8], m: &Mods) -> Option<Vec<u8>> {
    let (found, mut t) = match st.env.lookup(name, m.timestring)? {
        Value::Bytes(b) => (Some(b), 0),
        Value::Time(0) => return None,
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
            return Some((now - t).to_string().into_bytes());
        }
        if m.difference {
            return text::time_difference(now, t);
        }
        if m.pretty {
            return pretty_time(st, now, t);
        }
        let tm = st.host.local_time(t)?;
        return match &m.time_format {
            Some(f) => Some(st.host.strftime(f, &tm, 8192).unwrap_or_default()),
            None => Some(text::ctime(&tm)),
        };
    }

    let mut found = match found {
        Some(f) => f,
        None => t.to_string().into_bytes(),
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

/// format_pretty_time without seconds.
fn pretty_time(st: &State<'_>, now: i64, t: i64) -> Option<Vec<u8>> {
    let now = now.max(t);
    let age = now - t;
    let (Some(now_tm), Some(tm)) = (st.host.local_time(now), st.host.local_time(t)) else {
        return Some(Vec::new());
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
    st.host.strftime(fmt, &tm, 8192)
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
        _ => return None,
    };
    let mut use_fp = false;
    let mut prec = 0;
    if args.len() >= 2 && args[1].contains(&b'f') {
        use_fp = true;
        prec = 2;
    }
    if args.len() >= 3 {
        prec = text::strtonum(&args[2], -MAX_PRECISION, MAX_PRECISION)?;
    }
    let (l, rt) = r.choose()?;
    let left = expand1(st, l);
    let right = expand1(st, rt);
    let mut ml = text::strtod(&left)?;
    let mut mr = text::strtod(&right)?;
    if !use_fp {
        ml = text::to_long_long(ml);
        mr = text::to_long_long(mr);
    }
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    let result = match op {
        Op::Add => ml + mr,
        Op::Sub => ml - mr,
        Op::Mul => ml * mr,
        Op::Div => ml / mr,
        Op::Mod => ml % mr,
        Op::Eq => b((ml - mr).abs() < 1e-9),
        Op::Ne => b((ml - mr).abs() > 1e-9),
        Op::Gt => b(ml > mr),
        Op::Ge => b(ml >= mr),
        Op::Lt => b(ml < mr),
        Op::Le => b(ml <= mr),
    };
    if use_fp {
        Some(text::print_float(prec, result))
    } else {
        Some(text::print_float(prec, text::to_long_long(result)))
    }
}
