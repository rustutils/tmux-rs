//! Random formats from a small grammar.
//!
//! The generator builds formats out of literals, escapes, short aliases,
//! variables, modifiers with arguments, conditionals, comparisons, the
//! boolean operators and loops, nested up to a depth. It draws from a
//! seeded PRNG, so the same seed and count always give the same formats
//! and a run can be repeated.
//!
//! It only uses variables from [`VARIABLES`], which leaves out what
//! depends on time, pids, the host, the tty or the socket. It never
//! writes `%` (so `T:` has nothing to pass to strftime), `;` at the end
//! (the command line parser would take it as a command separator), `#(`
//! (jobs) or the `t` modifier.

/// A small PRNG (SplitMix64). Good enough to pick grammar rules and
/// stable across platforms and Rust versions, which a crate's generator
/// does not promise.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// Creates a generator from a seed.
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// Returns the next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Returns a number below `n`, which must not be zero.
    pub fn below(&mut self, n: usize) -> usize {
        debug_assert!(n > 0, "below(0) has no answer");
        (self.next_u64() % n as u64) as usize
    }

    /// Picks one item of a slice, which must not be empty.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// Returns true with a chance of `n` in `d`.
    pub fn chance(&mut self, n: usize, d: usize) -> bool {
        self.below(d) < n
    }
}

/// Variables the generator uses. They have the same value on both servers
/// when they are set up the same way. User options, plain options, an
/// environment variable, an unset name and the empty name are mixed in.
pub const VARIABLES: &[&str] = &[
    "session_name",
    "session_id",
    "session_windows",
    "session_attached",
    "session_grouped",
    "session_group",
    "session_group_size",
    "session_active",
    "session_marked",
    "session_path",
    "window_name",
    "window_index",
    "window_id",
    "window_panes",
    "window_flags",
    "window_raw_flags",
    "window_active",
    "window_layout",
    "window_width",
    "window_height",
    "window_zoomed_flag",
    "window_linked",
    "window_bigger",
    "pane_index",
    "pane_id",
    "pane_title",
    "pane_width",
    "pane_height",
    "pane_active",
    "pane_dead",
    "pane_in_mode",
    "pane_flags",
    "pane_current_command",
    "pane_start_command",
    "pane_current_path",
    "pane_left",
    "pane_top",
    "pane_at_top",
    "pane_at_left",
    "pane_marked",
    "pane_synchronized",
    "cursor_x",
    "cursor_y",
    "history_size",
    "history_limit",
    "alternate_on",
    "server_sessions",
    "buffer_name",
    "loop_index",
    "@plain",
    "@num",
    "@neg",
    "@float",
    "@path",
    "@utf8",
    "@empty",
    "@dotdot",
    "@fmt",
    "@punct",
    "@hashbrace",
    "@long",
    "@spaces",
    "@shell",
    "@colour",
    "@wopt",
    "@popt",
    "@sopt",
    "status-left",
    "status",
    "base-index",
    "mode-keys",
    "status-position",
    "FOO",
    "NOSUCHENV",
    "nosuchvar",
    "",
];

/// Short aliases that do not depend on the host (`#H` and `#h` do).
const ALIASES: &[&str] = &["#S", "#W", "#I", "#P", "#D", "#F", "#T"];

/// Escapes and a lone `#` before something that is not an alias.
const ESCAPES: &[&str] = &["##", "#,", "#}", "#{", "# ", "#x", "#["];

/// Characters for literals. The awkward ones (`,`, `{`, `}`, `#`, `:`)
/// are rarer, so most formats still parse.
const PLAIN: &[&str] = &[
    "a", "b", "x", "Z", "0", "1", "5", "-", "_", ".", "/", " ", "é", "☕", "日",
];
const AWKWARD: &[&str] = &[",", "{", "}", "#", ":", "|", "\\", "'", "\"", "$", "&"];

/// Numbers used as modifier arguments, including bad ones and ones past
/// 3.8's limits.
const NUMBERS: &[&str] = &[
    "0", "1", "2", "3", "5", "10", "-1", "-3", "-10", "100", "101", "10000", "10001", "x", "",
];

/// Operators for `e`. `%` is left out, see the module docs.
const ARITH: &[&str] = &["+", "-", "*", "/", "m", "==", "!=", "<", ">", "<=", ">="];

/// Generates formats.
pub struct Generator {
    rng: Rng,
    max_depth: usize,
}

impl Generator {
    /// Creates a generator for a seed.
    pub fn new(seed: u64) -> Self {
        Generator {
            rng: Rng::new(seed),
            max_depth: 3,
        }
    }

    /// Generates `count` formats.
    pub fn formats(&mut self, count: usize) -> Vec<String> {
        (0..count).map(|_| self.format()).collect()
    }

    /// Generates one format. It is never empty and never ends with `;`.
    pub fn format(&mut self) -> String {
        loop {
            let depth = 1 + self.rng.below(self.max_depth);
            let s = self.sequence(depth, true);
            if !s.is_empty() && !s.ends_with(';') && !s.contains("#(") {
                return s;
            }
        }
    }

    /// One to three items in a row.
    fn sequence(&mut self, depth: usize, top: bool) -> String {
        let n = 1 + self.rng.below(3);
        (0..n).map(|_| self.item(depth, top)).collect()
    }

    /// One item. At depth zero only leaves.
    fn item(&mut self, depth: usize, top: bool) -> String {
        if depth == 0 {
            return match self.rng.below(5) {
                0 => self.literal(top),
                1 => self.rng.pick(ESCAPES).to_string(),
                2 => self.rng.pick(ALIASES).to_string(),
                _ => self.variable(),
            };
        }
        match self.rng.below(20) {
            0..=1 => self.literal(top),
            2 => self.rng.pick(ESCAPES).to_string(),
            3 => self.rng.pick(ALIASES).to_string(),
            4..=6 => self.variable(),
            7..=11 => self.modified(depth),
            12..=13 => self.conditional(depth),
            14..=15 => self.comparison(depth),
            16 => self.boolean(depth),
            17 => self.looped(depth),
            18 => self.arithmetic(depth),
            _ => self.malformed(depth),
        }
    }

    /// A short literal. Inside arguments awkward characters are rarer,
    /// because a raw comma or brace changes the structure around it.
    fn literal(&mut self, top: bool) -> String {
        let n = 1 + self.rng.below(4);
        let awkward = if top { 4 } else { 12 };
        (0..n)
            .map(|_| {
                if self.rng.below(awkward) == 0 {
                    *self.rng.pick(AWKWARD)
                } else {
                    *self.rng.pick(PLAIN)
                }
            })
            .collect()
    }

    /// `#{name}`.
    fn variable(&mut self) -> String {
        format!("#{{{}}}", self.rng.pick(VARIABLES))
    }

    /// An argument of a modifier or operator: a literal, a variable or a
    /// nested format.
    fn argument(&mut self, depth: usize) -> String {
        match self.rng.below(4) {
            0 => self.literal(false),
            1 => self.variable(),
            2 => self.rng.pick(NUMBERS).to_string(),
            _ => self.sequence(depth.saturating_sub(1), false),
        }
    }

    /// What a value modifier is applied to: mostly a variable name, which
    /// is what `#{mods:name}` looks up, sometimes a nested format.
    fn subject(&mut self, depth: usize) -> String {
        if self.rng.chance(3, 4) {
            self.rng.pick(VARIABLES).to_string()
        } else {
            self.sequence(depth.saturating_sub(1), false)
        }
    }

    /// One modifier that changes a single value, with its arguments.
    fn value_modifier(&mut self) -> String {
        let n = |g: &mut Self| g.rng.pick(NUMBERS).to_string();
        match self.rng.below(24) {
            0 => "l".into(),
            1 => "b".into(),
            2 => "d".into(),
            3 => "n".into(),
            4 => "w".into(),
            5 => "E".into(),
            6 => "T".into(),
            7 => "a".into(),
            8 => self
                .rng
                .pick(&["q", "q/s", "q/a", "q/h", "q/e"])
                .to_string(),
            9 => format!("={}", n(self)),
            10 => {
                let marker = self.rng.pick(&["...", "", ">", "é", "#,"]).to_string();
                format!("=/{}/{}", n(self), marker)
            }
            11 => format!("p{}", n(self)),
            12 => {
                let pat = self
                    .rng
                    .pick(&["a", "e", "[0-9]", "(.)", "^", "$", ".*", "x|y", "é"]);
                let rep = self.rng.pick(&["", "X", "\\1", "#,", "<&>"]);
                let flags = self.rng.pick(&["", "i", "g"]);
                let d = self.rng.pick(&["/", "|", ","]);
                format!("s{d}{pat}{d}{rep}{d}{flags}")
            }
            13 => "!".into(),
            14 => "!!".into(),
            15 => self.rng.pick(&["c", "c/f", "c/b"]).to_string(),
            16 => self.rng.pick(&["N", "N/s", "N/w"]).to_string(),
            17 => self.rng.pick(&["C", "C/r", "C/i"]).to_string(),
            18 => self.rng.pick(&["I/e", "I/f", "I/c"]).to_string(),
            19 => "R".into(),
            20 => "A".into(),
            21 => "e".into(),
            22 => self.rng.pick(&["m", "<", ">"]).to_string(),
            _ => format!("={};p{}", n(self), n(self)),
        }
    }

    /// `#{mod;mod:subject}`, one or two modifiers.
    fn modified(&mut self, depth: usize) -> String {
        let mut mods = self.value_modifier();
        if self.rng.chance(1, 4) {
            mods.push(';');
            mods.push_str(&self.value_modifier());
        }
        format!("#{{{mods}:{}}}", self.subject(depth))
    }

    /// `#{?cond,a,b}` with optional else-if pairs and an optional else.
    fn conditional(&mut self, depth: usize) -> String {
        let mut s = format!("#{{?{}", self.condition(depth));
        s.push(',');
        s.push_str(&self.argument(depth));
        for _ in 0..self.rng.below(3) {
            s.push(',');
            s.push_str(&self.condition(depth));
            s.push(',');
            s.push_str(&self.argument(depth));
        }
        if self.rng.chance(2, 3) {
            s.push(',');
            s.push_str(&self.argument(depth));
        }
        s.push('}');
        s
    }

    /// A condition: a variable name or a nested format.
    fn condition(&mut self, depth: usize) -> String {
        if self.rng.chance(1, 2) {
            self.rng.pick(VARIABLES).to_string()
        } else {
            self.argument(depth)
        }
    }

    /// `#{op:a,b}` for the comparisons, `m` and `R`.
    fn comparison(&mut self, depth: usize) -> String {
        let op = self.rng.pick(&[
            "==", "!=", "<", ">", "<=", ">=", "m", "m/r", "m/i", "m/ri", "R", "m/z",
        ]);
        format!(
            "#{{{op}:{},{}}}",
            self.argument(depth),
            self.argument(depth)
        )
    }

    /// `#{||:a,b,...}`, `#{&&:...}`, `#{!:a}` or `#{!!:a}`.
    fn boolean(&mut self, depth: usize) -> String {
        let op = *self.rng.pick(&["||", "&&", "!", "!!"]);
        let n = if matches!(op, "!" | "!!") {
            1
        } else {
            self.rng.below(4)
        };
        let args: Vec<String> = (0..n).map(|_| self.argument(depth)).collect();
        format!("#{{{op}:{}}}", args.join(","))
    }

    /// `#{e|op|:a,b}` with optional float flag and precision.
    fn arithmetic(&mut self, depth: usize) -> String {
        let op = self.rng.pick(ARITH);
        let tail = match self.rng.below(4) {
            0 => "f|".to_string(),
            1 => format!("f|{}|", self.rng.pick(&["0", "2", "4", "100", "101"])),
            _ => String::new(),
        };
        let a = self.number_argument(depth);
        let b = self.number_argument(depth);
        format!("#{{e|{op}|{tail}:{a},{b}}}")
    }

    /// A number or something that expands to one.
    fn number_argument(&mut self, depth: usize) -> String {
        match self.rng.below(4) {
            0 => self.argument(depth),
            1 => self
                .rng
                .pick(&["#{@num}", "#{@neg}", "#{@float}", "#{window_index}"])
                .to_string(),
            _ => self
                .rng
                .pick(&["0", "1", "7", "-3", "2.5", "1e3", "x", "", "99999999999"])
                .to_string(),
        }
    }

    /// A loop over sessions, windows, panes, clients, options or the
    /// environment, with an optional second format for the current item.
    fn looped(&mut self, depth: usize) -> String {
        let l = self.rng.pick(&[
            "S", "S/n", "S/nr", "S/i", "S/r", "W", "W/n", "W/nr", "W/r", "P", "P/r", "P/i", "L",
            "O/p", "V", "V/s",
        ]);
        let inner = depth.saturating_sub(1);
        let mut s = format!("#{{{l}:{}", self.sequence(inner, false));
        if self.rng.chance(1, 2) {
            s.push(',');
            s.push_str(&self.sequence(inner, false));
        }
        s.push('}');
        s
    }

    /// Something broken: an unclosed brace, a modifier without a colon,
    /// a stray separator.
    fn malformed(&mut self, depth: usize) -> String {
        match self.rng.below(6) {
            0 => format!("#{{{}", self.rng.pick(VARIABLES)),
            1 => format!("#{{{}", self.value_modifier()),
            2 => format!("#{{?{}", self.argument(depth)),
            3 => format!("#{{{};}}", self.value_modifier()),
            4 => format!("#{{:{}}}", self.argument(depth)),
            _ => format!("#{{{}:", self.value_modifier()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_formats() {
        let a = Generator::new(42).formats(200);
        let b = Generator::new(42).formats(200);
        assert_eq!(a, b);
    }

    #[test]
    fn different_seed_different_formats() {
        let a = Generator::new(1).formats(50);
        let b = Generator::new(2).formats(50);
        assert_ne!(a, b);
    }

    #[test]
    fn known_sequence() {
        // Pins the PRNG, so a change to it (which would change every
        // seeded run) is noticed.
        let mut r = Rng::new(0);
        assert_eq!(r.next_u64(), 0xe220_a839_7b1d_cdaf);
    }

    #[test]
    fn formats_avoid_what_the_tool_cannot_compare() {
        for f in Generator::new(7).formats(2000) {
            assert!(!f.is_empty());
            assert!(!f.ends_with(';'), "{f}");
            assert!(!f.contains('%'), "{f}");
            assert!(!f.contains("#("), "{f}");
            assert!(!f.contains("#H") && !f.contains("#h"), "{f}");
        }
    }
}
