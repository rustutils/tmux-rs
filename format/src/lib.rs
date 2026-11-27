//! The format engine: parsing and expanding tmux's `#{}` formats as tmux
//! 3.8 does, byte for byte.
//!
//! A format is parsed into a tree ([`parse()`]) that keeps the raw bytes
//! of every node next to its parsed form, and the tree is expanded. The
//! [`Engine`] caches parsed formats by their bytes, so a status line is
//! parsed once and expanded many times. Values, names and output are
//! bytes, so names that are not UTF-8 survive.
//!
//! The engine has no global state and reads nothing from the server
//! itself. What it needs comes through two traits:
//!
//! - [`Host`], one per server: the clock, local time and strftime,
//!   regular expressions (POSIX, through tmux-regsub in tmux-rs), fnmatch,
//!   colour names and the fuzzy matcher.
//! - [`FormatEnv`], one per expansion (C's format_tree): variables by
//!   name, the items of a loop, `#()` job output, the pane search and the
//!   attached client's terminal.
//!
//! # Where C's semantics shape the tree
//!
//! - Arguments of modifiers are unescaped and expanded before anything
//!   else (format_build_modifiers). The tree stores each argument parsed
//!   and expands all of them first, in order.
//! - What a modifier does can depend on what its argument expands to:
//!   `N/x` and `I` without `f`, `c` or `e` are no modifier at all, and
//!   `t`, `q`, `=`, `p` and the loop sort flags read the expanded
//!   argument. So the evaluator ports C's switch over the expanded
//!   arguments, and the text after the modifiers is kept raw. Each way of
//!   reading it (whole, split at the first comma, split at every comma,
//!   the conditional, unescaped for `l`) is parsed the first time it is
//!   needed and kept in the node.
//! - `E:` re-expands a value, `s///` expands its already expanded
//!   arguments again, and job output is expanded. These parse at run
//!   time, through the cache.
//! - Expanding with time (`T:`, status lines, display -p) runs strftime
//!   over the raw text of every expansion level before it is scanned, so
//!   `%%` inside a conditional branch is formatted twice. In that mode
//!   each level formats its node's raw text and parses the result (cached).
//! - The conditional looks up the raw condition text as a name first and
//!   only expands it when that finds nothing.
//! - `#{` or `#(` without an end, and a replacement that fails (a loop
//!   without a session, a comparison without a comma), stop the whole
//!   expansion with what was output so far.
//! - The time limit (100 milliseconds of [`Host::timer_ms`]) is checked
//!   where C checks it: at each expansion level and wherever C scans for
//!   the end of something at run time (a `#{`, a comma of a choice, a
//!   condition or an operator). The tree was scanned when it was parsed,
//!   so the evaluator checks at the same points instead. Past the limit
//!   each level expands to nothing and the next `#{` stops the output.
//! - With [`FormatEnv::logging`] on, the engine logs the lines C's
//!   format_log does (display -v), through [`FormatEnv::log`].
//!
//! # Drawing
//!
//! [`draw::draw`] lays out an expanded format with its `#[]` styles
//! (format-draw.c): alignments, lists with markers and focus, fill and
//! ranges. It returns the cells to write and the ranges, the caller
//! writes them to its screen. Styles are parsed by the caller through
//! [`draw::DrawHost`].

#![forbid(unsafe_code)]

pub mod draw;
mod eval;
mod parse;
mod text;

#[cfg(test)]
mod cases38;
#[cfg(test)]
mod tests;

pub use eval::{Engine, ExpandFlags};
pub use parse::{Format, parse, skip, unescape};
pub use text::{format_width, pad, trim_left, trim_right};

/// A broken down local time (struct tm).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tm {
    /// Seconds, 0 to 60.
    pub sec: i32,
    /// Minutes, 0 to 59.
    pub min: i32,
    /// Hours, 0 to 23.
    pub hour: i32,
    /// Day of the month, 1 to 31.
    pub mday: i32,
    /// Month, 0 to 11.
    pub mon: i32,
    /// Years since 1900.
    pub year: i32,
    /// Day of the week, 0 is Sunday.
    pub wday: i32,
    /// Day of the year, 0 to 365.
    pub yday: i32,
    /// Daylight saving time flag.
    pub isdst: i32,
}

/// What a lookup found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// A string value.
    Bytes(Vec<u8>),
    /// A time (FORMAT_TABLE_TIME callbacks and format_add_tv entries).
    /// Zero counts as not found, as in C.
    Time(i64),
}

/// The kinds of loop modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopKind {
    /// `S:`, the sessions.
    Sessions,
    /// `W:`, the windows of the session.
    Windows,
    /// `P:`, the panes of the window.
    Panes,
    /// `L:`, the clients.
    Clients,
    /// `O:`, the options of a scope (the flags pick it).
    Options,
    /// `V:`, the variables of an environment (the flags pick it).
    Environ,
}

/// The sort orders a loop can ask for (sort.c's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    /// The order the items are kept in.
    Order,
    /// By index.
    Index,
    /// By name.
    Name,
    /// By activity time.
    Activity,
    /// By creation time.
    Creation,
    /// By z order (panes).
    Z,
}

/// How to sort a loop. C keeps this in a static that the last loop
/// modifier of a replacement sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopSort {
    /// The order.
    pub order: SortOrder,
    /// Whether it is reversed.
    pub reversed: bool,
}

/// One item of a loop.
pub struct LoopChild<'a> {
    /// The environment of the item (its own format tree in C), holding
    /// everything C adds to it: loop_index, loop_last_flag, the neighbour
    /// variables of `W:` (window_before_active, window_after_active and
    /// the next_ and prev_ variables), the option_ and environ_
    /// variables of `O:` and `V:`, and the session, window, pane and
    /// client variables of format_defaults. Its other methods answer for
    /// the item as C's tree for it would (the client of `L:` is the
    /// item). For the log, [`FormatEnv::loop_message`] gives the line C
    /// logs for the item.
    pub env: Box<dyn FormatEnv + 'a>,
    /// Whether it is the current item, which uses the second format of
    /// `S:`, `W:` and `P:`.
    pub active: bool,
}

/// Which name `N:` checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    /// A window of the session.
    Window,
    /// A session.
    Session,
}

/// What `I` asks about the attached client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientQuery {
    /// `I/c`: whether its terminal has a capability.
    Capability,
    /// `I/f`: whether its terminal has a feature.
    Feature,
    /// `I/e`: a variable of its environment.
    Environ,
}

/// What the engine needs from the server, the same for every expansion.
pub trait Host {
    /// The current time in seconds.
    fn now(&self) -> i64;

    /// A monotonic clock in milliseconds (get_timer), for the time limit
    /// and the animation frames of `A`.
    fn timer_ms(&self) -> u64;

    /// localtime_r. None when the time cannot be converted.
    fn local_time(&self, t: i64) -> Option<Tm>;

    /// strftime into C's buffer of `max` bytes. None when the result does
    /// not fit or is empty (strftime returned 0).
    fn strftime(&self, fmt: &[u8], tm: &Tm, max: usize) -> Option<Vec<u8>>;

    /// fnmatch(3), with FNM_CASEFOLD when `casefold`.
    fn fnmatch(&self, pattern: &[u8], text: &[u8], casefold: bool) -> bool;

    /// A POSIX extended regular expression search. False when the pattern
    /// does not compile, as in C.
    fn regex_match(&self, pattern: &[u8], text: &[u8], icase: bool) -> bool;

    /// regsub: replaces every match. None when the pattern does not
    /// compile, then the value stays as it was.
    fn regex_sub(&self, pattern: &[u8], with: &[u8], text: &[u8], icase: bool) -> Option<Vec<u8>>;

    /// colour_fromstring: a colour by name, None when it is not one.
    fn colour(&self, name: &[u8]) -> Option<i32>;

    /// colour_force_rgb: the colour as 0xRRGGBB, None when it has no RGB
    /// value (default, terminal).
    fn colour_rgb(&self, colour: i32) -> Option<u32>;

    /// fuzzy_match: which of the `width` columns of `text` matched
    /// `pattern`, None when it does not match.
    fn fuzzy_match(&self, pattern: &[u8], text: &[u8], width: u32) -> Option<Vec<bool>>;
}

/// What one expansion looks things up in, C's format_tree.
pub trait FormatEnv {
    /// Looks up a variable by name, as format_find does before it applies
    /// modifiers. `for_time` is set for `t:`, which skips the environment.
    fn lookup(&self, name: &[u8], for_time: bool) -> Option<Value>;

    /// The items of a loop, sorted. `flags` is C's loop_flags: the
    /// expanded argument of the last `O` or `V` that has exactly one
    /// argument, empty otherwise and for the other loops. The env reads
    /// it as format_loop_options and format_loop_environ do (empty means
    /// `s`). `sort` is what the last of `S`, `W`, `P` and `L` asked for,
    /// C's defaults without one. None when there is nothing to loop over
    /// and the replacement fails (`W:` without a session, `P:` without a
    /// window). `O:` and `V:` without a scope give an empty list.
    ///
    /// When several loop modifiers are given, the engine picks one in
    /// C's order: `S`, `W`, `P`, `L`, `O`, then `V`. Only `S`, `W` and
    /// `P` split their format at the first comma for the active item.
    fn children(&self, kind: LoopKind, sort: LoopSort, flags: &[u8]) -> Option<Vec<LoopChild<'_>>>;

    /// Whether `#()` is disabled for this tree (FORMAT_NOJOBS).
    fn jobs_disabled(&self) -> bool {
        false
    }

    /// The last output of the job for `cmd` (the raw text, the key), now
    /// that it expands to `expanded`. The env starts or restarts the job
    /// as format_job_get does. None when there is no output yet.
    fn job(&self, cmd: &[u8], expanded: &[u8]) -> Option<Vec<u8>> {
        let _ = (cmd, expanded);
        None
    }

    /// Whether a window of the session (or a session) has this name. None
    /// for a window without a session, which fails the replacement. C
    /// fails before it expands the name, so the engine first asks with an
    /// empty name to find out. None for a session counts as false.
    fn name_exists(&self, kind: NameKind, name: &[u8]) -> Option<bool> {
        let _ = (kind, name);
        None
    }

    /// `C:`, the line of a search in the pane (0 when nothing matched),
    /// None without a pane.
    fn search_pane(&self, text: &[u8], regex: bool, icase: bool) -> Option<u32> {
        let _ = (text, regex, icase);
        None
    }

    /// `I`: None without an attached client with a terminal, which gives
    /// an empty value. Capabilities and features answer `1` or `0`, a
    /// variable of the environment its value or nothing. `name` is the
    /// raw text after the modifiers, not expanded, as in C.
    fn client_query(&self, query: ClientQuery, name: &[u8]) -> Option<Vec<u8>> {
        let _ = (query, name);
        None
    }

    /// colour_toescape for the client of this tree (`c/f` and `c/b`).
    /// Without one C assumes a terminal with 256 and RGB colours. None
    /// when colour_toescape gives nothing, which gives an empty value.
    fn colour_escape(&self, colour: i32, background: bool) -> Option<Vec<u8>> {
        let _ = (colour, background);
        None
    }

    /// Whether this is a status line (FORMAT_STATUS), where `A` animates.
    /// Asked once, of the env passed to [`Engine::expand`].
    fn is_status(&self) -> bool {
        false
    }

    /// Asks for the status line to be redrawn while an animation of `A`
    /// runs (format_cycle_start_timer).
    fn start_cycle_timer(&self) {}

    /// Whether to log each step of the expansion (C's format_logging:
    /// display -v or a server log). Asked once, of the env passed to
    /// [`Engine::expand`], when the expansion starts.
    fn logging(&self) -> bool {
        false
    }

    /// One line of the log, the text C's format_log formats, and `depth`,
    /// C's es->loop at that point. C prints it with display -v as "#",
    /// then `depth` spaces (at most 10), then the message. Only called on
    /// the env passed to [`Engine::expand`], also for lines from inside a
    /// loop, and only when [`FormatEnv::logging`] said yes.
    fn log(&self, depth: u32, message: &[u8]) {
        let _ = (depth, message);
    }

    /// For a child of a loop: the line C logs before it expands the
    /// child, such as "session loop: $1", "window loop: 0 @1", "pane
    /// loop: %2", "client loop: name", "option loop: name",
    /// `option loop: name[key]` or "environment loop: NAME". Only asked
    /// when logging. None logs nothing.
    fn loop_message(&self) -> Option<Vec<u8>> {
        None
    }

    /// For a child of a loop that is built lazily: whether it is the
    /// current item, asked just before the child is expanded. None uses
    /// [`LoopChild::active`]. C builds each loop item from its list only
    /// when it reaches it, and a nested loop of the same kind can change
    /// that list (3.8's sort_get functions share one static array), so an
    /// env that copies this decides late.
    fn loop_active(&self) -> Option<bool> {
        None
    }

    /// The ID of the pane (without the %), for the log line of `C:`.
    /// Only asked when logging and [`FormatEnv::search_pane`] found a
    /// pane.
    fn pane_id(&self) -> Option<u32> {
        None
    }
}
