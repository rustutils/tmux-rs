// Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
//
// Permission to use, copy, modify, and distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
// IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
// OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

//! Option trees, options.c from tmux 3.8 in safe Rust.
//!
//! An [`Options`] tree holds entries by name. An entry either has a table
//! entry ([`OptionTableEntry`], from `options_table.rs`), which gives its
//! type, limits and default, or is a user option (`@name`), which is
//! always a string. Values are typed ([`Value`]): strings, numbers (also
//! keys, colours, flags and choices) and command lists. An array option
//! holds values by [`ArrayKey`], which is a number or, since 3.8, a
//! name. Numeric keys sort first, by number, then names by bytes.
//!
//! Trees are linked to a parent (a session's tree to the global session
//! options and so on). The store does not follow the link: lookups that
//! inherit take the chain of trees ([`chain_get`]) and setting a value
//! from a string takes the parents ([`Options::set_from_string`]).
//!
//! Some of what an option needs is not converted yet: parsing command
//! lists, keys, colours and styles, and checking shells and patterns.
//! [`OptionsExt`] supplies those, along with the command list and cached
//! style types, and a tag for the tree that owns an entry. The store
//! never calls out otherwise, takes the time as a parameter, and leaves
//! the side effects of a change (`options_push_changes`) to its caller.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::fmt;

/// Scope: not an option, or the scope lookup failed.
pub const OPTIONS_TABLE_NONE: i32 = 0;
/// Scope: server options.
pub const OPTIONS_TABLE_SERVER: i32 = 0x1;
/// Scope: session options.
pub const OPTIONS_TABLE_SESSION: i32 = 0x2;
/// Scope: window options.
pub const OPTIONS_TABLE_WINDOW: i32 = 0x4;
/// Scope: pane options.
pub const OPTIONS_TABLE_PANE: i32 = 0x8;

/// Flag: the option is an array.
pub const OPTIONS_TABLE_IS_ARRAY: i32 = 0x1;
/// Flag: the option is a hook.
pub const OPTIONS_TABLE_IS_HOOK: i32 = 0x2;
/// Flag: the option is a style, checked when set.
pub const OPTIONS_TABLE_IS_STYLE: i32 = 0x4;

/// The type of a table option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionType {
    String,
    Number,
    Key,
    Colour,
    Flag,
    Choice,
    Command,
}

impl OptionType {
    /// Whether values of this type are held as numbers.
    pub fn is_number(self) -> bool {
        matches!(
            self,
            Self::Number | Self::Key | Self::Colour | Self::Flag | Self::Choice
        )
    }
}

/// One option in the table: its name, type, scope and default.
#[derive(Debug)]
pub struct OptionTableEntry {
    pub name: &'static str,
    pub type_: OptionType,
    /// `OPTIONS_TABLE_SERVER` and the other scope bits.
    pub scope: i32,
    /// `OPTIONS_TABLE_IS_ARRAY` and the other flag bits.
    pub flags: i32,
    pub minimum: u32,
    pub maximum: u32,

    /// The values of a choice option, the value is the index.
    pub choices: &'static [&'static str],

    pub default_str: Option<&'static str>,
    pub default_num: i64,
    pub default_arr: Option<&'static [&'static str]>,

    /// What separates array items when the whole array is set, and what
    /// joins old and new value when a string is appended to. Arrays
    /// default to space and comma, strings to nothing.
    pub separator: Option<&'static str>,
    /// An fnmatch pattern string values must match.
    pub pattern: Option<&'static str>,

    /// The description shown in customize mode.
    pub text: Option<&'static str>,
    /// The unit shown after the value in customize mode.
    pub unit: Option<&'static str>,
}

impl OptionTableEntry {
    /// An entry with every field empty, for `..const_default()` in the
    /// table.
    pub const fn const_default() -> Self {
        Self {
            name: "",
            type_: OptionType::String,
            scope: 0,
            flags: 0,
            minimum: 0,
            maximum: 0,
            choices: &[],
            default_str: None,
            default_num: 0,
            default_arr: None,
            separator: None,
            pattern: None,
            text: None,
            unit: None,
        }
    }

    pub fn is_array(&self) -> bool {
        self.flags & OPTIONS_TABLE_IS_ARRAY != 0
    }

    pub fn is_hook(&self) -> bool {
        self.flags & OPTIONS_TABLE_IS_HOOK != 0
    }

    pub fn is_style(&self) -> bool {
        self.flags & OPTIONS_TABLE_IS_STYLE != 0
    }

    /// The index of `value` in the choices.
    pub fn find_choice(&self, value: &str) -> Result<usize, OptionsError> {
        self.choices
            .iter()
            .position(|&c| c == value)
            .ok_or_else(|| OptionsError::UnknownValue(value.to_string()))
    }

    /// The default value as text, as show-options would print it.
    pub fn default_to_string<X: OptionsExt>(&self) -> String {
        match self.type_ {
            OptionType::String | OptionType::Command => {
                self.default_str.unwrap_or("").to_string()
            }
            OptionType::Number => self.default_num.to_string(),
            OptionType::Key => X::print_key(self.default_num),
            OptionType::Colour => X::print_colour(self.default_num),
            OptionType::Flag => if self.default_num != 0 { "on" } else { "off" }.to_string(),
            OptionType::Choice => choice_name(self, self.default_num),
        }
    }
}

/// The name of choice `n`, or the number if it is out of range.
fn choice_name(oe: &OptionTableEntry, n: i64) -> String {
    usize::try_from(n)
        .ok()
        .and_then(|i| oe.choices.get(i))
        .map_or_else(|| n.to_string(), |c| (*c).to_string())
}

/// Old option names and the names they map to.
pub const OPTIONS_OTHER_NAMES: [(&str, &str); 5] = [
    ("display-panes-color", "display-panes-colour"),
    ("display-panes-active-color", "display-panes-active-colour"),
    ("clock-mode-color", "clock-mode-colour"),
    ("cursor-color", "cursor-colour"),
    ("pane-colors", "pane-colours"),
];

/// The current name for an old option name, or the name itself.
pub fn options_map_name(name: &str) -> &str {
    OPTIONS_OTHER_NAMES
        .iter()
        .find(|(from, _)| *from == name)
        .map_or(name, |(_, to)| to)
}

/// What the store needs from code that is not converted yet.
///
/// The methods are called without a tree borrowed by the caller, but may
/// be called while the store is in the middle of a change.
pub trait OptionsExt {
    /// A parsed command list, owned by the option. Dropping it frees it.
    type Command;
    /// A parsed style, cached on string options.
    type Style;
    /// A tag for the tree an entry belongs to.
    type Owner: Copy + PartialEq;

    fn parse_command(s: &str) -> Result<Self::Command, String>;
    fn print_command(cmd: &Self::Command) -> String;
    /// A key code from its name, or None if it is not a key.
    fn parse_key(s: &str) -> Option<i64>;
    fn print_key(key: i64) -> String;
    /// A colour from its name, or None if it is not a colour.
    fn parse_colour(s: &str) -> Option<i64>;
    fn print_colour(colour: i64) -> String;
    /// Whether `value` may be used as default-shell.
    fn is_suitable_shell(value: &CStr) -> bool;
    /// Whether `value` matches the fnmatch `pattern`.
    fn matches_pattern(pattern: &str, value: &CStr) -> bool;
    /// Whether `value` parses as a style.
    fn is_valid_style(value: &CStr) -> bool;
}

/// An error from setting an option. The text is tmux's message.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OptionsError {
    #[error("not an array")]
    NotAnArray,
    #[error("bad array key: {0}")]
    BadArrayKey(String),
    #[error("bad colour: {0}")]
    BadColour(String),
    #[error("wrong array type")]
    WrongArrayType,
    /// A command list did not parse, with the parser's message.
    #[error("{0}")]
    Command(String),
    #[error("empty value")]
    EmptyValue,
    #[error("bad option name")]
    BadOptionName,
    /// A number out of range or not a number: "too small", "too large"
    /// or "invalid", then the value.
    #[error("value is {0}: {1}")]
    Number(&'static str, String),
    #[error("bad key: {0}")]
    BadKey(String),
    #[error("bad value: {0}")]
    BadValue(String),
    #[error("unknown value: {0}")]
    UnknownValue(String),
    #[error("not a suitable shell: {0}")]
    NotAShell(String),
    #[error("value is invalid: {0}")]
    Invalid(String),
    #[error("invalid style: {0}")]
    InvalidStyle(String),
}

/// The key of an array item. Numbers sort before names, numbers by
/// value and names by bytes, as 3.8's `options_array_cmp`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArrayKey {
    Number(u32),
    Name(String),
}

impl ArrayKey {
    /// Reads a key as 3.8's `options_array_correct_key`: digits only is a
    /// number (so `01` is 1), up to `u32::MAX`, anything else is a name.
    /// An empty key or a number that is too large is an error.
    pub fn parse(key: &str) -> Result<Self, OptionsError> {
        if key.is_empty() {
            return Err(OptionsError::BadArrayKey(String::new()));
        }
        if !key.bytes().all(|b| b.is_ascii_digit()) {
            return Ok(Self::Name(key.to_string()));
        }
        key.parse::<u32>()
            .map(Self::Number)
            .map_err(|_| OptionsError::BadArrayKey(key.to_string()))
    }
}

impl From<u32> for ArrayKey {
    fn from(n: u32) -> Self {
        Self::Number(n)
    }
}

impl fmt::Display for ArrayKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(n) => write!(f, "{n}"),
            Self::Name(s) => f.write_str(s),
        }
    }
}

/// Splits `name[key]` into the name and key, as `parse_option_name`. Returns
/// None for an empty name, a `[` without a `]` at the very end, an empty
/// key or a bad key.
pub fn parse_option_name(name: &str) -> Option<(String, Option<ArrayKey>)> {
    if name.is_empty() {
        return None;
    }
    let Some(open) = name.find('[') else {
        return Some((name.to_string(), None));
    };
    let rest = &name[open + 1..];
    let close = rest.find(']')?;
    if close + 1 != rest.len() || close == 0 {
        return None;
    }
    let key = ArrayKey::parse(&rest[..close]).ok()?;
    Some((name[..open].to_string(), Some(key)))
}

/// Why [`match_option_name`] found no option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchError {
    /// More than one option starts with the name.
    Ambiguous,
    /// No option has the name, or it does not parse.
    Invalid,
}

/// Finds the option a name refers to: a user option, an exact name, an
/// old name or the only option the name is a prefix of. Returns the full
/// name and the array key.
pub fn match_option_name(
    table: &[OptionTableEntry],
    s: &str,
) -> Result<(String, Option<ArrayKey>), MatchError> {
    let (parsed, key) = parse_option_name(s).ok_or(MatchError::Invalid)?;
    if parsed.starts_with('@') {
        return Ok((parsed, key));
    }
    let name = options_map_name(&parsed);
    let mut found: Option<&OptionTableEntry> = None;
    for oe in table {
        if oe.name == name {
            found = Some(oe);
            break;
        }
        if oe.name.starts_with(name) {
            if found.is_some() {
                return Err(MatchError::Ambiguous);
            }
            found = Some(oe);
        }
    }
    found
        .map(|oe| (oe.name.to_string(), key))
        .ok_or(MatchError::Invalid)
}

/// The table entry with exactly this name.
pub fn search_table(
    table: &'static [OptionTableEntry],
    name: &str,
) -> Option<&'static OptionTableEntry> {
    table.iter().find(|oe| oe.name == name)
}

/// A value: a string, a number (numbers, keys, colours, flags and
/// choices) or a command list. A command option whose default did not
/// parse has no command list.
pub enum Value<C> {
    String(CString),
    Number(i64),
    Command(Option<C>),
}

impl<C> Value<C> {
    pub fn as_string(&self) -> Option<&CStr> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<i64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_command(&self) -> Option<&C> {
        match self {
            Self::Command(c) => c.as_ref(),
            _ => None,
        }
    }
}

/// Makes a C string from bytes, cut at the first NUL as C would.
fn c_string(bytes: &[u8]) -> CString {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    CString::new(&bytes[..end]).unwrap_or_default()
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

enum EntryValue<C> {
    One(Value<C>),
    Array(BTreeMap<ArrayKey, Value<C>>),
}

/// One option in a tree.
pub struct OptionEntry<X: OptionsExt> {
    name: String,
    table: Option<&'static OptionTableEntry>,
    owner: X::Owner,
    value: EntryValue<X::Command>,
    /// The parsed style and whether it may be used without parsing
    /// again (the value has no formats).
    style: Option<X::Style>,
    cached: bool,
    fire_count: u32,
    fire_time: i64,
}

impl<X: OptionsExt> OptionEntry<X> {
    fn new(name: &str, owner: X::Owner) -> Self {
        Self {
            name: name.to_string(),
            table: None,
            owner,
            value: EntryValue::One(Value::String(CString::default())),
            style: None,
            cached: false,
            fire_count: 0,
            fire_time: 0,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The table entry, None for a user option.
    pub fn table(&self) -> Option<&'static OptionTableEntry> {
        self.table
    }

    /// The tag of the tree the entry is in.
    pub fn owner(&self) -> X::Owner {
        self.owner
    }

    pub fn is_array(&self) -> bool {
        self.table.is_some_and(OptionTableEntry::is_array)
    }

    /// User options and table string options are strings.
    pub fn is_string(&self) -> bool {
        self.table.is_none_or(|oe| oe.type_ == OptionType::String)
    }

    pub fn is_number(&self) -> bool {
        self.table.is_some_and(|oe| oe.type_.is_number())
    }

    pub fn is_command(&self) -> bool {
        self.table.is_some_and(|oe| oe.type_ == OptionType::Command)
    }

    /// The value of an option that is not an array.
    pub fn value(&self) -> Option<&Value<X::Command>> {
        match &self.value {
            EntryValue::One(v) => Some(v),
            EntryValue::Array(_) => None,
        }
    }

    pub fn string(&self) -> Option<&CStr> {
        self.value().and_then(Value::as_string)
    }

    pub fn number(&self) -> Option<i64> {
        self.value().and_then(Value::as_number)
    }

    /// The array item with this key.
    pub fn array_get(&self, key: &ArrayKey) -> Option<&Value<X::Command>> {
        match &self.value {
            EntryValue::Array(a) => a.get(key),
            EntryValue::One(_) => None,
        }
    }

    /// The array items in key order. Nothing for an option that is not
    /// an array.
    pub fn array_items(&self) -> impl Iterator<Item = (&ArrayKey, &Value<X::Command>)> {
        let array = match &self.value {
            EntryValue::Array(a) => Some(a),
            EntryValue::One(_) => None,
        };
        array.into_iter().flatten()
    }

    fn array_mut(&mut self) -> Option<&mut BTreeMap<ArrayKey, Value<X::Command>>> {
        match &mut self.value {
            EntryValue::Array(a) => Some(a),
            EntryValue::One(_) => None,
        }
    }

    /// Removes every array item.
    pub fn array_clear(&mut self) {
        if let Some(a) = self.array_mut() {
            a.clear();
        }
    }

    /// Sets array item `key` from a string, or removes it if `value` is
    /// None. With `append`, a string is added to the end of the item.
    pub fn array_set(
        &mut self,
        key: &ArrayKey,
        value: Option<&[u8]>,
        append: bool,
    ) -> Result<(), OptionsError> {
        let Some(oe) = self.table.filter(|oe| oe.is_array()) else {
            return Err(OptionsError::NotAnArray);
        };
        let Some(array) = self.array_mut() else {
            return Err(OptionsError::NotAnArray);
        };
        let Some(value) = value else {
            array.remove(key);
            return Ok(());
        };
        let new = match oe.type_ {
            OptionType::Command => {
                let cmd = X::parse_command(&lossy(value)).map_err(OptionsError::Command)?;
                Value::Command(Some(cmd))
            }
            OptionType::String => match array.get(key) {
                Some(Value::String(old)) if append => {
                    let mut bytes = old.as_bytes().to_vec();
                    bytes.extend_from_slice(value);
                    Value::String(c_string(&bytes))
                }
                _ => Value::String(c_string(value)),
            },
            OptionType::Colour => {
                let text = lossy(value);
                let colour = X::parse_colour(&text).ok_or(OptionsError::BadColour(text))?;
                Value::Number(colour)
            }
            _ => return Err(OptionsError::WrongArrayType),
        };
        array.insert(key.clone(), new);
        Ok(())
    }

    /// The first number that is not a key yet.
    fn array_free_number(&self) -> Option<u32> {
        (0..u32::MAX).find(|&i| self.array_get(&ArrayKey::Number(i)).is_none())
    }

    /// Adds the items of `s` at the first free numbers. `s` is split at
    /// any of the separator characters, empty items are skipped. An
    /// option with an empty separator takes `s` as one item.
    pub fn array_assign(&mut self, s: &[u8]) -> Result<(), OptionsError> {
        let separator = self
            .table
            .and_then(|oe| oe.separator)
            .unwrap_or(" ,")
            .as_bytes();
        if s.is_empty() {
            return Ok(());
        }
        if separator.is_empty() {
            let i = self.array_free_number().unwrap_or(u32::MAX);
            return self.array_set(&ArrayKey::Number(i), Some(s), false);
        }
        for item in s.split(|b| separator.contains(b)) {
            if item.is_empty() {
                continue;
            }
            let Some(i) = self.array_free_number() else {
                break;
            };
            self.array_set(&ArrayKey::Number(i), Some(item), false)?;
        }
        Ok(())
    }

    fn value_to_string(&self, value: &Value<X::Command>, numeric: bool) -> String {
        match value {
            Value::Command(cmd) => cmd.as_ref().map(X::print_command).unwrap_or_default(),
            Value::String(s) => lossy(s.as_bytes()),
            Value::Number(number) => {
                let n = *number;
                match self.table.map(|oe| (oe, oe.type_)) {
                    Some((_, OptionType::Key)) => X::print_key(n),
                    Some((_, OptionType::Colour)) => X::print_colour(n),
                    Some((_, OptionType::Flag)) if !numeric => {
                        if n != 0 { "on" } else { "off" }.to_string()
                    }
                    Some((oe, OptionType::Choice)) => choice_name(oe, n),
                    _ => n.to_string(),
                }
            }
        }
    }

    /// The value as text, as show-options prints it. For an array, the
    /// item with `key`, or every item joined with spaces if there is no
    /// key. `numeric` prints flags as 1 and 0.
    pub fn to_string(&self, key: Option<&ArrayKey>, numeric: bool) -> String {
        match (&self.value, key) {
            (EntryValue::Array(a), None) => a
                .values()
                .map(|v| self.value_to_string(v, numeric))
                .collect::<Vec<_>>()
                .join(" "),
            (EntryValue::Array(a), Some(key)) => a
                .get(key)
                .map(|v| self.value_to_string(v, numeric))
                .unwrap_or_default(),
            (EntryValue::One(v), _) => self.value_to_string(v, numeric),
        }
    }

    /// The style parsed from the value, if it is still valid.
    pub fn cached_style(&mut self) -> Option<&mut X::Style> {
        if self.cached {
            self.style.as_mut()
        } else {
            None
        }
    }

    /// Keeps a parsed style. With `cache` it is returned by
    /// [`Self::cached_style`] until the value changes.
    pub fn store_style(&mut self, style: X::Style, cache: bool) -> &mut X::Style {
        self.cached = cache;
        self.style.insert(style)
    }

    /// Notes that the hook in this option ran at `now`.
    pub fn hook_fired(&mut self, now: i64) {
        self.fire_count = self.fire_count.wrapping_add(1);
        self.fire_time = now;
    }

    /// How often the hook ran.
    pub fn fire_count(&self) -> u32 {
        self.fire_count
    }

    /// When the hook last ran, 0 if never.
    pub fn fire_time(&self) -> i64 {
        self.fire_time
    }
}

/// An option tree.
///
/// Entries are boxed, so the address of an entry stays the same while
/// other entries are added or removed. The adapter relies on that.
pub struct Options<X: OptionsExt> {
    table: &'static [OptionTableEntry],
    owner: X::Owner,
    entries: BTreeMap<String, Box<OptionEntry<X>>>,
}

impl<X: OptionsExt> Options<X> {
    /// An empty tree whose options are described by `table`. Its entries
    /// are tagged with `owner`.
    pub fn new(table: &'static [OptionTableEntry], owner: X::Owner) -> Self {
        Self {
            table,
            owner,
            entries: BTreeMap::new(),
        }
    }

    /// Changes the tag of the tree and its entries.
    pub fn set_owner(&mut self, owner: X::Owner) {
        self.owner = owner;
        for e in self.entries.values_mut() {
            e.owner = owner;
        }
    }

    /// The name an entry is stored under: the name, or its new name.
    fn key_of(&self, name: &str) -> Option<String> {
        if self.entries.contains_key(name) {
            return Some(name.to_string());
        }
        let mapped = options_map_name(name);
        self.entries
            .contains_key(mapped)
            .then(|| mapped.to_string())
    }

    /// The entry in this tree, not looking at parents.
    pub fn get_only(&self, name: &str) -> Option<&OptionEntry<X>> {
        self.entries
            .get(name)
            .or_else(|| self.entries.get(options_map_name(name)))
            .map(|e| &**e)
    }

    pub fn get_only_mut(&mut self, name: &str) -> Option<&mut OptionEntry<X>> {
        let key = self.key_of(name)?;
        self.entries.get_mut(&key).map(|e| &mut **e)
    }

    /// Every entry, in name order.
    #[cfg(test)]
    pub fn entries(&self) -> impl Iterator<Item = &OptionEntry<X>> {
        self.entries.values().map(|e| &**e)
    }

    pub fn entries_mut(&mut self) -> impl Iterator<Item = &mut OptionEntry<X>> {
        self.entries.values_mut().map(|e| &mut **e)
    }

    /// Adds an entry, replacing one with the same name.
    fn add(&mut self, name: &str) -> &mut OptionEntry<X> {
        self.remove(name);
        let entry = Box::new(OptionEntry::new(name, self.owner));
        self.entries.entry(name.to_string()).or_insert(entry)
    }

    /// Removes an entry.
    pub fn remove(&mut self, name: &str) {
        if let Some(key) = self.key_of(name) {
            self.entries.remove(&key);
        }
    }

    /// Adds an entry for a table option with no value (an empty array,
    /// an empty string or 0), replacing one with the same name.
    pub fn add_empty(&mut self, oe: &'static OptionTableEntry) -> &mut OptionEntry<X> {
        let o = self.add(oe.name);
        o.table = Some(oe);
        o.value = if oe.is_array() {
            EntryValue::Array(BTreeMap::new())
        } else if oe.type_ == OptionType::String {
            EntryValue::One(Value::String(CString::default()))
        } else if oe.type_ == OptionType::Command {
            EntryValue::One(Value::Command(None))
        } else {
            EntryValue::One(Value::Number(0))
        };
        o
    }

    /// Adds an entry for a table option with its default value,
    /// replacing one with the same name.
    pub fn set_default(&mut self, oe: &'static OptionTableEntry) -> &mut OptionEntry<X> {
        let o = self.add_empty(oe);
        if oe.is_array() {
            match oe.default_arr {
                None => _ = o.array_assign(oe.default_str.unwrap_or("").as_bytes()),
                Some(items) => {
                    for (i, item) in (0u32..).zip(items) {
                        _ = o.array_set(&ArrayKey::Number(i), Some(item.as_bytes()), false);
                    }
                }
            }
            return o;
        }
        o.value = EntryValue::One(match oe.type_ {
            OptionType::String => Value::String(c_string(oe.default_str.unwrap_or("").as_bytes())),
            OptionType::Command => {
                Value::Command(X::parse_command(oe.default_str.unwrap_or("")).ok())
            }
            _ => Value::Number(oe.default_num),
        });
        o
    }

    /// The table entry for a table option that is not in this tree yet.
    /// Panics for an unknown name, as C's `options_parent_table_entry`.
    fn table_entry(&self, name: &str) -> &'static OptionTableEntry {
        let name = options_map_name(name);
        let Some(oe) = search_table(self.table, name) else {
            panic!("{name} not in parent options");
        };
        oe
    }

    /// Sets a string option. With `append` and an existing string in
    /// this tree, the value is added to the end, after the option's
    /// separator (none for user options). A table option missing from
    /// this tree is added with its default first.
    ///
    /// Panics if the option is not a string, as C does.
    pub fn set_string(&mut self, name: &str, value: &[u8], append: bool) -> &mut OptionEntry<X> {
        let mut new = value.to_vec();
        if append
            && let Some(o) = self.get_only(name)
            && o.is_string()
        {
            let separator = if name.starts_with('@') {
                ""
            } else {
                o.table.and_then(|oe| oe.separator).unwrap_or("")
            };
            let mut joined = o.string().map(|s| s.to_bytes().to_vec()).unwrap_or_default();
            joined.extend_from_slice(separator.as_bytes());
            joined.extend_from_slice(value);
            new = joined;
        }
        if self.get_only(name).is_none() {
            if name.starts_with('@') {
                self.add(name);
            } else {
                let oe = self.table_entry(name);
                self.set_default(oe);
            }
        }
        let Some(o) = self.get_only_mut(name) else {
            unreachable!("option {name} was just added");
        };
        assert!(o.is_string(), "option {name} is not a string");
        o.value = EntryValue::One(Value::String(c_string(&new)));
        o.cached = false;
        o
    }

    /// Sets a number option, adding it with its default first if it is
    /// not in this tree.
    ///
    /// Panics for a user option or an option that is not a number.
    pub fn set_number(&mut self, name: &str, value: i64) -> &mut OptionEntry<X> {
        assert!(!name.starts_with('@'), "user option {name} must be a string");
        if self.get_only(name).is_none() {
            let oe = self.table_entry(name);
            self.set_default(oe);
        }
        let Some(o) = self.get_only_mut(name) else {
            unreachable!("option {name} was just added");
        };
        assert!(o.is_number(), "option {name} is not a number");
        o.value = EntryValue::One(Value::Number(value));
        o
    }

    /// Sets a command option, adding it with its default first if it is
    /// not in this tree.
    ///
    /// Panics for a user option or an option that is not a command.
    pub fn set_command(&mut self, name: &str, value: X::Command) -> &mut OptionEntry<X> {
        assert!(!name.starts_with('@'), "user option {name} must be a string");
        if self.get_only(name).is_none() {
            let oe = self.table_entry(name);
            self.set_default(oe);
        }
        let Some(o) = self.get_only_mut(name) else {
            unreachable!("option {name} was just added");
        };
        assert!(o.is_command(), "option {name} is not a command");
        o.value = EntryValue::One(Value::Command(Some(value)));
        o
    }

    /// Sets an option that is not an array from a string, checking it
    /// for its type. `parents` are the trees this one inherits from,
    /// nearest first: toggling a flag or choice without a value, and
    /// restoring a string that fails its check, use the inherited value.
    ///
    /// `oe` is the option's table entry, None for a user option.
    pub fn set_from_string(
        &mut self,
        parents: &[&Self],
        oe: Option<&'static OptionTableEntry>,
        name: &str,
        value: Option<&[u8]>,
        append: bool,
    ) -> Result<(), OptionsError> {
        let type_ = match oe {
            Some(oe) => {
                if value.is_none() && !matches!(oe.type_, OptionType::Flag | OptionType::Choice) {
                    return Err(OptionsError::EmptyValue);
                }
                oe.type_
            }
            None if !name.starts_with('@') => return Err(OptionsError::BadOptionName),
            None => OptionType::String,
        };
        let text = value.map(lossy).unwrap_or_default();

        match type_ {
            OptionType::String => {
                let old = self
                    .inherited(parents, name)
                    .and_then(OptionEntry::string)
                    .map(|s| s.to_bytes().to_vec())
                    .unwrap_or_default();
                let new = self.set_string(name, value.unwrap_or_default(), append);
                let checked = new.string().map(|s| check_string::<X>(oe, s));
                if let Some(Err(e)) = checked {
                    self.set_string(name, &old, false);
                    return Err(e);
                }
            }
            OptionType::Number => {
                let oe = oe.unwrap_or_else(|| unreachable!("user options are strings"));
                let n = parse_number(&text, oe.minimum.into(), oe.maximum.into())
                    .map_err(|e| OptionsError::Number(e, text.clone()))?;
                self.set_number(name, n);
            }
            OptionType::Key => {
                let key = X::parse_key(&text).ok_or_else(|| OptionsError::BadKey(text.clone()))?;
                self.set_number(name, key);
            }
            OptionType::Colour => {
                let colour = X::parse_colour(&text).ok_or_else(|| OptionsError::BadColour(text.clone()))?;
                self.set_number(name, colour);
            }
            OptionType::Flag => {
                let flag = if text.is_empty() {
                    self.inherited_number(parents, name) == 0
                } else if text == "1"
                    || text.eq_ignore_ascii_case("on")
                    || text.eq_ignore_ascii_case("yes")
                {
                    true
                } else if text == "0"
                    || text.eq_ignore_ascii_case("off")
                    || text.eq_ignore_ascii_case("no")
                {
                    false
                } else {
                    return Err(OptionsError::BadValue(text));
                };
                self.set_number(name, i64::from(flag));
            }
            OptionType::Choice => {
                let oe = oe.unwrap_or_else(|| unreachable!("user options are strings"));
                let choice = if value.is_none() {
                    let n = self.inherited_number(parents, name);
                    if n < 2 { i64::from(n == 0) } else { n }
                } else {
                    oe.find_choice(&text)? as i64
                };
                self.set_number(name, choice);
            }
            OptionType::Command => {
                let cmd = X::parse_command(&text).map_err(OptionsError::Command)?;
                self.set_command(name, cmd);
            }
        }
        Ok(())
    }

    /// The entry in this tree or the nearest parent that has it.
    fn inherited<'a>(&'a self, parents: &[&'a Self], name: &str) -> Option<&'a OptionEntry<X>> {
        std::iter::once(self)
            .chain(parents.iter().copied())
            .find_map(|oo| oo.get_only(name))
    }

    fn inherited_number(&self, parents: &[&Self], name: &str) -> i64 {
        self.inherited(parents, name)
            .and_then(OptionEntry::number)
            .unwrap_or(0)
    }

    /// Unsets an option: a global option (`global`) with a table entry
    /// goes back to its default, others are removed. With a key, only
    /// that array item is removed.
    pub fn remove_or_default(
        &mut self,
        name: &str,
        key: Option<&ArrayKey>,
        global: bool,
    ) -> Result<(), OptionsError> {
        let Some(o) = self.get_only_mut(name) else {
            return Ok(());
        };
        match key {
            Some(key) => o.array_set(key, None, false),
            None => {
                match o.table {
                    Some(oe) if global => {
                        self.set_default(oe);
                    }
                    _ => self.remove(name),
                }
                Ok(())
            }
        }
    }
}

/// The entry for `name` in the first tree of `chain` that has it. The
/// chain is a tree followed by its parents, nearest first.
pub fn chain_get<'a, X: OptionsExt>(
    chain: &[&'a Options<X>],
    name: &str,
) -> Option<&'a OptionEntry<X>> {
    chain.iter().find_map(|oo| oo.get_only(name))
}

/// The checks a table string option's value must pass, in C's order.
fn check_string<X: OptionsExt>(
    oe: Option<&'static OptionTableEntry>,
    value: &CStr,
) -> Result<(), OptionsError> {
    let Some(oe) = oe else {
        return Ok(());
    };
    let text = || lossy(value.to_bytes());
    if oe.name == "default-shell" && !X::is_suitable_shell(value) {
        return Err(OptionsError::NotAShell(text()));
    }
    if let Some(pattern) = oe.pattern
        && !X::matches_pattern(pattern, value)
    {
        return Err(OptionsError::Invalid(text()));
    }
    if oe.is_style() && !text().contains("#{") && !X::is_valid_style(value) {
        return Err(OptionsError::InvalidStyle(text()));
    }
    Ok(())
}

/// Reads a number as strtonum: optional leading space and sign, then
/// digits to the end. Errors are "invalid", "too small" and "too large".
fn parse_number(s: &str, min: i64, max: i64) -> Result<i64, &'static str> {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    if min > max || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err("invalid");
    }
    match t.parse::<i64>() {
        Ok(n) if n < min => Err("too small"),
        Ok(n) if n > max => Err("too large"),
        Ok(n) => Ok(n),
        Err(_) if t.starts_with('-') => Err("too small"),
        Err(_) => Err("too large"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Commands are their text, keys and colours are numbers written in
    /// decimal, styles are valid unless they say "bad".
    struct T;

    impl OptionsExt for T {
        type Command = String;
        type Style = String;
        type Owner = u32;

        fn parse_command(s: &str) -> Result<String, String> {
            if s.contains("bad") {
                Err(format!("parse error: {s}"))
            } else {
                Ok(s.to_string())
            }
        }
        fn print_command(cmd: &String) -> String {
            cmd.clone()
        }
        fn parse_key(s: &str) -> Option<i64> {
            s.strip_prefix('K').and_then(|n| n.parse().ok())
        }
        fn print_key(key: i64) -> String {
            format!("K{key}")
        }
        fn parse_colour(s: &str) -> Option<i64> {
            s.strip_prefix("colour").and_then(|n| n.parse().ok())
        }
        fn print_colour(colour: i64) -> String {
            format!("colour{colour}")
        }
        fn is_suitable_shell(value: &CStr) -> bool {
            value.to_bytes().starts_with(b"/")
        }
        fn matches_pattern(_pattern: &str, value: &CStr) -> bool {
            value.to_bytes().contains(&b'x')
        }
        fn is_valid_style(value: &CStr) -> bool {
            !value.to_bytes().starts_with(b"bad")
        }
    }

    static CHOICES: [&str; 3] = ["off", "on", "both"];
    static DEFAULT_ARR: [&str; 2] = ["one", "two"];

    static TABLE: [OptionTableEntry; 12] = [
        OptionTableEntry {
            name: "base-index",
            type_: OptionType::Number,
            maximum: 100,
            default_num: 0,
            ..OptionTableEntry::const_default()
        },
        OptionTableEntry {
            name: "bell",
            type_: OptionType::Choice,
            choices: &CHOICES,
            default_num: 1,
            ..OptionTableEntry::const_default()
        },
        OptionTableEntry {
            name: "colours",
            type_: OptionType::Colour,
            flags: OPTIONS_TABLE_IS_ARRAY,
            default_str: Some(""),
            ..OptionTableEntry::const_default()
        },
        OptionTableEntry {
            name: "default-shell",
            type_: OptionType::String,
            default_str: Some("/bin/sh"),
            ..OptionTableEntry::const_default()
        },
        OptionTableEntry {
            name: "flag",
            type_: OptionType::Flag,
            default_num: 1,
            ..OptionTableEntry::const_default()
        },
        OptionTableEntry {
            name: "hook",
            type_: OptionType::Command,
            flags: OPTIONS_TABLE_IS_ARRAY | OPTIONS_TABLE_IS_HOOK,
            default_str: Some(""),
            separator: Some(""),
            ..OptionTableEntry::const_default()
        },
        OptionTableEntry {
            name: "key",
            type_: OptionType::Key,
            default_num: 2,
            ..OptionTableEntry::const_default()
        },
        OptionTableEntry {
            name: "list",
            type_: OptionType::String,
            flags: OPTIONS_TABLE_IS_ARRAY,
            default_arr: Some(&DEFAULT_ARR),
            ..OptionTableEntry::const_default()
        },
        OptionTableEntry {
            name: "pane-colours",
            type_: OptionType::Colour,
            flags: OPTIONS_TABLE_IS_ARRAY,
            default_str: Some(""),
            ..OptionTableEntry::const_default()
        },
        OptionTableEntry {
            name: "pattern",
            type_: OptionType::String,
            pattern: Some("*x*"),
            default_str: Some("x"),
            ..OptionTableEntry::const_default()
        },
        OptionTableEntry {
            name: "status",
            type_: OptionType::String,
            default_str: Some("left"),
            separator: Some(","),
            ..OptionTableEntry::const_default()
        },
        OptionTableEntry {
            name: "status-style",
            type_: OptionType::String,
            flags: OPTIONS_TABLE_IS_STYLE,
            default_str: Some("fg=red"),
            ..OptionTableEntry::const_default()
        },
    ];

    fn tree() -> Options<T> {
        let mut oo = Options::new(&TABLE, 1);
        for oe in &TABLE {
            oo.set_default(oe);
        }
        oo
    }

    fn key(s: &str) -> ArrayKey {
        ArrayKey::parse(s).unwrap()
    }

    fn keys(o: &OptionEntry<T>) -> Vec<String> {
        o.array_items().map(|(k, _)| k.to_string()).collect()
    }

    #[test]
    fn array_keys_parse_as_3_8() {
        assert_eq!(key("01"), ArrayKey::Number(1));
        assert_eq!(key("0"), ArrayKey::Number(0));
        assert_eq!(key("4294967295"), ArrayKey::Number(u32::MAX));
        assert_eq!(key("x"), ArrayKey::Name("x".into()));
        assert_eq!(key("-1"), ArrayKey::Name("-1".into()));
        assert_eq!(key("a b"), ArrayKey::Name("a b".into()));
        assert!(ArrayKey::parse("").is_err());
        assert_eq!(
            ArrayKey::parse("4294967296"),
            Err(OptionsError::BadArrayKey("4294967296".into()))
        );
    }

    #[test]
    fn array_keys_sort_numbers_first() {
        let mut v = [key("x"), key("10"), key("B"), key("2"), key("-1")];
        v.sort();
        let s: Vec<String> = v.iter().map(ToString::to_string).collect();
        assert_eq!(s, ["2", "10", "-1", "B", "x"]);
    }

    #[test]
    fn parse_names_and_keys() {
        assert_eq!(parse_option_name("status"), Some(("status".into(), None)));
        assert_eq!(
            parse_option_name("list[01]"),
            Some(("list".into(), Some(ArrayKey::Number(1))))
        );
        assert_eq!(
            parse_option_name("list[a b]"),
            Some(("list".into(), Some(key("a b"))))
        );
        assert_eq!(
            parse_option_name("@x[y]"),
            Some(("@x".into(), Some(key("y"))))
        );
        assert_eq!(parse_option_name(""), None);
        assert_eq!(parse_option_name("list[]"), None);
        assert_eq!(parse_option_name("list[1"), None);
        assert_eq!(parse_option_name("list[1]x"), None);
        assert_eq!(parse_option_name("list[a]b]"), None);
        assert_eq!(parse_option_name("list[99999999999]"), None);
    }

    #[test]
    fn match_exact_prefix_old_name_and_ambiguous() {
        assert_eq!(match_option_name(&TABLE, "bell"), Ok(("bell".into(), None)));
        assert_eq!(match_option_name(&TABLE, "ba"), Ok(("base-index".into(), None)));
        // "status" is exact although "status-style" starts with it.
        assert_eq!(match_option_name(&TABLE, "status"), Ok(("status".into(), None)));
        assert_eq!(match_option_name(&TABLE, "sta"), Err(MatchError::Ambiguous));
        assert_eq!(
            match_option_name(&TABLE, "pane-colors[3]"),
            Ok(("pane-colours".into(), Some(ArrayKey::Number(3))))
        );
        assert_eq!(match_option_name(&TABLE, "nothing"), Err(MatchError::Invalid));
        assert_eq!(match_option_name(&TABLE, "list[]"), Err(MatchError::Invalid));
        assert_eq!(match_option_name(&TABLE, "@user"), Ok(("@user".into(), None)));
    }

    #[test]
    fn search_is_exact() {
        assert_eq!(search_table(&TABLE, "bell").unwrap().name, "bell");
        assert!(search_table(&TABLE, "bel").is_none());
        assert!(search_table(&TABLE, "pane-colors").is_none());
    }

    #[test]
    fn defaults() {
        let oo = tree();
        assert_eq!(oo.get_only("base-index").unwrap().number(), Some(0));
        assert_eq!(oo.get_only("status").unwrap().string().unwrap(), c"left");
        assert_eq!(keys(oo.get_only("list").unwrap()), ["0", "1"]);
        assert_eq!(oo.get_only("list").unwrap().to_string(None, false), "one two");
        assert_eq!(keys(oo.get_only("hook").unwrap()), Vec::<String>::new());
        assert_eq!(TABLE[1].default_to_string::<T>(), "on");
        assert_eq!(TABLE[6].default_to_string::<T>(), "K2");
        assert_eq!(TABLE[4].default_to_string::<T>(), "on");
    }

    #[test]
    fn get_only_maps_old_names() {
        let oo = tree();
        assert_eq!(oo.get_only("pane-colors").unwrap().name(), "pane-colours");
        assert!(oo.get_only("nothing").is_none());
    }

    #[test]
    fn get_falls_through_the_chain() {
        let global = tree();
        let mut local = Options::<T>::new(&TABLE, 2);
        assert!(local.get_only("base-index").is_none());
        let found = chain_get(&[&local, &global], "base-index").unwrap();
        assert_eq!(found.owner(), 1);
        local.set_number("base-index", 5);
        let found = chain_get(&[&local, &global], "base-index").unwrap();
        assert_eq!((found.owner(), found.number()), (2, Some(5)));
    }

    #[test]
    fn set_owner_tags_entries() {
        let mut oo = tree();
        oo.set_owner(9);
        assert_eq!(oo.get_only("bell").unwrap().owner(), 9);
        assert_eq!(oo.set_string("@new", b"x", false).owner(), 9);
    }

    #[test]
    fn entries_are_in_name_order() {
        let mut oo = tree();
        oo.set_string("@b", b"1", false);
        oo.set_string("@a", b"1", false);
        let names: Vec<&str> = oo.entries().map(OptionEntry::name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        assert_eq!(names[0], "@a");
    }

    #[test]
    fn set_string_appends_with_separator() {
        let mut oo = tree();
        oo.set_string("status", b"right", true);
        assert_eq!(oo.get_only("status").unwrap().string().unwrap(), c"left,right");
        oo.set_string("@u", b"a", false);
        oo.set_string("@u", b"b", true);
        assert_eq!(oo.get_only("@u").unwrap().string().unwrap(), c"ab");
    }

    #[test]
    fn set_string_adds_missing_option_with_default_first() {
        let mut local = Options::<T>::new(&TABLE, 2);
        // Appending to an option missing from this tree does not append.
        local.set_string("status", b"x", true);
        assert_eq!(local.get_only("status").unwrap().string().unwrap(), c"x");
        assert!(local.get_only("status").unwrap().table().is_some());
    }

    #[test]
    fn set_string_keeps_bytes_and_cuts_at_nul() {
        let mut oo = tree();
        oo.set_string("@b", b"\xff\xfe", false);
        assert_eq!(oo.get_only("@b").unwrap().string().unwrap().to_bytes(), b"\xff\xfe");
        oo.set_string("@b", b"ab\0cd", false);
        assert_eq!(oo.get_only("@b").unwrap().string().unwrap(), c"ab");
    }

    #[test]
    #[should_panic(expected = "user option @x must be a string")]
    fn set_number_on_user_option_panics() {
        tree().set_number("@x", 1);
    }

    #[test]
    fn from_string_number_limits() {
        let mut oo = tree();
        oo.set_from_string(&[], Some(&TABLE[0]), "base-index", Some(b"7"), false)
            .unwrap();
        assert_eq!(oo.get_only("base-index").unwrap().number(), Some(7));
        let e = |v: &[u8]| {
            tree()
                .set_from_string(&[], Some(&TABLE[0]), "base-index", Some(v), false)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(e(b"101"), "value is too large: 101");
        assert_eq!(e(b"-1"), "value is too small: -1");
        assert_eq!(e(b"x"), "value is invalid: x");
        assert_eq!(e(b"1 "), "value is invalid: 1 ");
        assert_eq!(e(b"99999999999999999999"), "value is too large: 99999999999999999999");
        assert_eq!(parse_number(" +3", 0, 5), Ok(3));
    }

    #[test]
    fn from_string_flag() {
        let mut oo = tree();
        for (v, want) in [("off", 0), ("YES", 1), ("0", 0), ("On", 1), ("no", 0)] {
            oo.set_from_string(&[], Some(&TABLE[4]), "flag", Some(v.as_bytes()), false)
                .unwrap();
            assert_eq!(oo.get_only("flag").unwrap().number(), Some(want), "{v}");
        }
        // No value toggles, also an empty value.
        oo.set_from_string(&[], Some(&TABLE[4]), "flag", None, false).unwrap();
        assert_eq!(oo.get_only("flag").unwrap().number(), Some(1));
        oo.set_from_string(&[], Some(&TABLE[4]), "flag", Some(b""), false)
            .unwrap();
        assert_eq!(oo.get_only("flag").unwrap().number(), Some(0));
        assert_eq!(
            oo.set_from_string(&[], Some(&TABLE[4]), "flag", Some(b"maybe"), false)
                .unwrap_err()
                .to_string(),
            "bad value: maybe"
        );
    }

    #[test]
    fn toggling_uses_the_inherited_value() {
        let global = tree();
        let mut local = Options::<T>::new(&TABLE, 2);
        local
            .set_from_string(&[&global], Some(&TABLE[4]), "flag", None, false)
            .unwrap();
        assert_eq!(local.get_only("flag").unwrap().number(), Some(0));
        // A choice of 0 or 1 toggles, 2 and up stays.
        local
            .set_from_string(&[&global], Some(&TABLE[1]), "bell", None, false)
            .unwrap();
        assert_eq!(local.get_only("bell").unwrap().number(), Some(0));
        local.set_number("bell", 2);
        local
            .set_from_string(&[&global], Some(&TABLE[1]), "bell", None, false)
            .unwrap();
        assert_eq!(local.get_only("bell").unwrap().number(), Some(2));
    }

    #[test]
    fn from_string_choice_key_colour_and_command() {
        let mut oo = tree();
        oo.set_from_string(&[], Some(&TABLE[1]), "bell", Some(b"both"), false)
            .unwrap();
        assert_eq!(oo.get_only("bell").unwrap().to_string(None, false), "both");
        let err = oo
            .set_from_string(&[], Some(&TABLE[1]), "bell", Some(b"loud"), false)
            .unwrap_err();
        assert_eq!(err.to_string(), "unknown value: loud");
        oo.set_from_string(&[], Some(&TABLE[6]), "key", Some(b"K7"), false)
            .unwrap();
        assert_eq!(oo.get_only("key").unwrap().to_string(None, false), "K7");
        let err = oo
            .set_from_string(&[], Some(&TABLE[6]), "key", Some(b"Q"), false)
            .unwrap_err();
        assert_eq!(err.to_string(), "bad key: Q");
        assert_eq!(
            oo.set_from_string(&[], Some(&TABLE[0]), "base-index", None, false),
            Err(OptionsError::EmptyValue)
        );
        assert_eq!(
            oo.set_from_string(&[], None, "plain", Some(b"x"), false),
            Err(OptionsError::BadOptionName)
        );
    }

    #[test]
    fn failed_string_check_restores_the_inherited_value() {
        let global = tree();
        let mut local = Options::<T>::new(&TABLE, 2);
        let err = local
            .set_from_string(&[&global], Some(&TABLE[11]), "status-style", Some(b"bad"), false)
            .unwrap_err();
        assert_eq!(err.to_string(), "invalid style: bad");
        // As C, the old value is now set in this tree.
        assert_eq!(
            local.get_only("status-style").unwrap().string().unwrap(),
            c"fg=red"
        );
        // Styles with formats are not checked.
        local
            .set_from_string(&[&global], Some(&TABLE[11]), "status-style", Some(b"bad#{x}"), false)
            .unwrap();
    }

    #[test]
    fn string_checks_in_order() {
        let mut oo = tree();
        let err = oo
            .set_from_string(&[], Some(&TABLE[3]), "default-shell", Some(b"sh"), false)
            .unwrap_err();
        assert_eq!(err.to_string(), "not a suitable shell: sh");
        let err = oo
            .set_from_string(&[], Some(&TABLE[9]), "pattern", Some(b"abc"), false)
            .unwrap_err();
        assert_eq!(err.to_string(), "value is invalid: abc");
        assert_eq!(oo.get_only("pattern").unwrap().string().unwrap(), c"x");
    }

    #[test]
    fn array_set_append_and_remove() {
        let mut oo = tree();
        let o = oo.get_only_mut("list").unwrap();
        o.array_set(&key("x"), Some(b"b"), false).unwrap();
        o.array_set(&key("x"), Some(b"c"), true).unwrap();
        assert_eq!(o.to_string(Some(&key("x")), false), "bc");
        o.array_set(&key("0"), None, false).unwrap();
        assert_eq!(keys(o), ["1", "x"]);
        assert_eq!(o.to_string(Some(&key("9")), false), "");
    }

    #[test]
    fn array_set_types() {
        let mut oo = tree();
        let o = oo.get_only_mut("colours").unwrap();
        o.array_set(&key("1"), Some(b"colour4"), false).unwrap();
        assert_eq!(o.array_get(&key("1")).unwrap().as_number(), Some(4));
        assert_eq!(
            o.array_set(&key("1"), Some(b"pink"), false),
            Err(OptionsError::BadColour("pink".into()))
        );
        let o = oo.get_only_mut("hook").unwrap();
        o.array_set(&key("0"), Some(b"run"), false).unwrap();
        assert_eq!(o.array_get(&key("0")).unwrap().as_command().unwrap(), "run");
        assert_eq!(
            o.array_set(&key("1"), Some(b"bad"), false),
            Err(OptionsError::Command("parse error: bad".into()))
        );
        let o = oo.get_only_mut("bell").unwrap();
        assert_eq!(o.array_set(&key("1"), Some(b"x"), false), Err(OptionsError::NotAnArray));
    }

    #[test]
    fn array_assign_takes_free_numbers() {
        let mut oo = tree();
        let o = oo.get_only_mut("list").unwrap();
        o.array_clear();
        o.array_set(&key("1"), Some(b"a"), false).unwrap();
        o.array_assign(b"p, q,,r").unwrap();
        assert_eq!(o.to_string(None, false), "p a q r");
        assert_eq!(keys(o), ["0", "1", "2", "3"]);
        // An empty separator takes the whole value as one item.
        let o = oo.get_only_mut("hook").unwrap();
        o.array_assign(b"a, b").unwrap();
        assert_eq!(o.to_string(None, false), "a, b");
    }

    #[test]
    fn remove_or_default() {
        let mut oo = tree();
        oo.set_number("base-index", 3);
        oo.remove_or_default("base-index", None, true).unwrap();
        assert_eq!(oo.get_only("base-index").unwrap().number(), Some(0));
        oo.remove_or_default("base-index", None, false).unwrap();
        assert!(oo.get_only("base-index").is_none());
        oo.remove_or_default("list", Some(&key("0")), true).unwrap();
        assert_eq!(keys(oo.get_only("list").unwrap()), ["1"]);
        // Missing options are fine.
        oo.remove_or_default("nothing", None, true).unwrap();
    }

    #[test]
    fn flags_print_as_numbers_when_asked() {
        let oo = tree();
        let o = oo.get_only("flag").unwrap();
        assert_eq!(o.to_string(None, false), "on");
        assert_eq!(o.to_string(None, true), "1");
    }

    #[test]
    fn style_cache_is_dropped_when_the_value_changes() {
        let mut oo = tree();
        let o = oo.get_only_mut("status-style").unwrap();
        assert!(o.cached_style().is_none());
        o.store_style("parsed".into(), true);
        assert_eq!(o.cached_style().unwrap(), "parsed");
        o.store_style("again".into(), false);
        assert!(o.cached_style().is_none());
        o.store_style("parsed".into(), true);
        oo.set_string("status-style", b"fg=blue", false);
        assert!(oo.get_only_mut("status-style").unwrap().cached_style().is_none());
    }

    #[test]
    fn hook_fire_count_and_time() {
        let mut oo = tree();
        let o = oo.get_only_mut("hook").unwrap();
        assert_eq!((o.fire_count(), o.fire_time()), (0, 0));
        o.hook_fired(100);
        o.hook_fired(105);
        assert_eq!((o.fire_count(), o.fire_time()), (2, 105));
    }

    #[test]
    fn entry_addresses_are_stable() {
        let mut oo = tree();
        let before: *const OptionEntry<T> = oo.get_only("status").unwrap();
        for i in 0..100 {
            oo.set_string(&format!("@{i}"), b"x", false);
        }
        let after: *const OptionEntry<T> = oo.get_only("status").unwrap();
        assert_eq!(before, after);
    }
}
