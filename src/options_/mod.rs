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

//! Options for the rest of tmux.
//!
//! [`store`] holds an option tree and its rules in safe Rust. This file
//! is the adapter: [`options`] is a tree plus the pointer to its parent,
//! held by the server (the three global trees) and by each session,
//! window and pane. The functions here keep options.c's names and
//! follow the parent pointers, so unconverted code keeps calling them.
//! [`COptions`] gives the store what is not converted yet: command lists,
//! keys, colours and styles.
//!
//! Pointers to entries ([`options_entry`]) stay valid until the entry is
//! removed or replaced, as in C. Array items are handed out as copies.
//! What has to happen after an option changes is in [`changes`].

pub mod changes;
pub mod store;

pub use self::changes::options_push_changes;
pub use self::store::{
    ArrayKey, MatchError, OptionEntry, OptionTableEntry, Options, OptionsError,
    OptionsExt, Value,
};

use crate::libc::fnmatch;
use crate::*;

/// A command list owned by an option. Dropping it frees it.
pub struct CmdList(NonNull<cmd_list>);

impl CmdList {
    /// The command list, valid while the option holds it.
    pub fn as_ptr(&self) -> *mut cmd_list {
        self.0.as_ptr()
    }
}

impl Drop for CmdList {
    fn drop(&mut self) {
        // SAFETY: the pointer came from cmd_parse_from_string and is owned
        // by this value, which drops one reference.
        unsafe { cmd_list_free(self.0.as_ptr()) }
    }
}

/// The parts of options that are still C-shaped code.
pub struct COptions;

impl OptionsExt for COptions {
    type Command = CmdList;
    type Style = style;
    type Owner = *mut options;
    type Monitor = crate::hook_monitor::HookMonitor;

    fn parse_command(s: &str) -> Result<CmdList, String> {
        // SAFETY: parsing a string does not touch option trees.
        match unsafe { cmd_parse_from_string(s, None) } {
            Ok(cmdlist) => NonNull::new(cmdlist)
                .map(CmdList)
                .ok_or_else(String::new),
            Err(error) => Err(error.to_string_lossy().into_owned()),
        }
    }

    fn print_command(cmd: &CmdList) -> String {
        // SAFETY: the command list is valid while the option holds it, and
        // cmd_list_print returns a new string, freed here.
        unsafe {
            let s = cmd_list_print(&*cmd.as_ptr(), 0);
            let out = cstr_to_string_lossy(s);
            free_(s);
            out
        }
    }

    fn parse_key(s: &str) -> Option<i64> {
        let s = CString::new(s).ok()?;
        // SAFETY: s is a valid C string for the call.
        let key = unsafe { key_string_lookup_string(s.as_ptr().cast()) };
        (key != KEYC_UNKNOWN).then_some(key as i64)
    }

    fn print_key(key: i64) -> String {
        key_string_lookup_key(key as u64, 0)
    }

    fn parse_colour(s: &str) -> Option<i64> {
        let colour = colour_fromstring(s);
        (colour != -1).then_some(i64::from(colour))
    }

    fn print_colour(colour: i64) -> String {
        colour_tostring(colour as i32).into_owned()
    }

    fn is_suitable_shell(value: &CStr) -> bool {
        // SAFETY: value is a valid C string for the call.
        unsafe { checkshell_(value.as_ptr().cast()) }
    }

    fn matches_pattern(pattern: &str, value: &CStr) -> bool {
        let Ok(pattern) = CString::new(pattern) else {
            return false;
        };
        // SAFETY: both are valid C strings for the call.
        unsafe { fnmatch(pattern.as_ptr().cast(), value.as_ptr().cast(), 0) == 0 }
    }

    fn is_valid_style(value: &CStr) -> bool {
        // SAFETY: sy is a local style and value a valid C string.
        unsafe {
            let mut sy: style = zeroed();
            style_parse(&mut sy, &GRID_DEFAULT_CELL, value.as_ptr().cast()) == 0
        }
    }
}

/// An entry in an option tree.
pub type options_entry = OptionEntry<COptions>;

/// An option tree and its parent, as C's struct options.
pub struct options {
    store: Options<COptions>,
    parent: *mut options,
}

/// The tree and the trees it inherits from, nearest first.
///
/// # Safety
/// `oo` and its parents must be valid, and nothing may change them while
/// the references are used.
unsafe fn options_chain<'a>(mut oo: *const options) -> Vec<&'a Options<COptions>> {
    unsafe {
        let mut chain = Vec::new();
        while !oo.is_null() {
            chain.push(&(*oo).store);
            oo = (*oo).parent;
        }
        chain
    }
}

/// A new empty tree with a parent, or null for a global tree.
pub unsafe fn options_create(parent: *mut options) -> *mut options {
    let oo = Box::into_raw(Box::new(options {
        store: Options::new(&OPTIONS_TABLE, null_mut()),
        parent,
    }));
    // SAFETY: oo was just allocated.
    unsafe { (*oo).store.set_owner(oo) };
    oo
}

/// Frees a tree and its entries.
pub unsafe fn options_free(oo: *mut options) {
    unsafe { drop(Box::from_raw(oo)) }
}

pub unsafe fn options_get_parent(oo: *mut options) -> *mut options {
    unsafe { (*oo).parent }
}

pub fn options_set_parent(oo: &mut options, parent: *mut options) {
    oo.parent = parent;
}

/// Every entry of a tree, in name order.
pub unsafe fn options_entries(oo: *mut options) -> Vec<*mut options_entry> {
    unsafe {
        (*oo)
            .store
            .entries_mut()
            .map(|e| e as *mut options_entry)
            .collect()
    }
}

/// The entry in this tree, not looking at the parents. Null if missing.
pub unsafe fn options_get_only(oo: *mut options, name: &str) -> *mut options_entry {
    unsafe {
        (*oo)
            .store
            .get_only_mut(name)
            .map_or(null_mut(), |e| e as *mut options_entry)
    }
}

/// The entry in this tree or the nearest parent. Null if missing.
pub unsafe fn options_get(oo: &mut options, name: &str) -> *mut options_entry {
    let mut tree: *mut options = oo;
    unsafe {
        while !tree.is_null() {
            let o = options_get_only(tree, name);
            if !o.is_null() {
                return o;
            }
            tree = (*tree).parent;
        }
        null_mut()
    }
}

unsafe fn options_get_const<'a>(oo: *const options, name: &str) -> Option<&'a options_entry> {
    unsafe { store::chain_get(&options_chain(oo), name) }
}

/// Adds an option with no value, replacing one with the same name.
pub unsafe fn options_empty(oo: *mut options, oe: *const options_table_entry) -> *mut options_entry {
    unsafe { (*oo).store.add_empty(&*oe) }
}

/// Adds an option with its default value, replacing one with the same
/// name.
pub unsafe fn options_default(oo: *mut options, oe: *const options_table_entry) -> *mut options_entry {
    unsafe { (*oo).store.set_default(&*oe) }
}

/// The default of a table option as text.
pub fn options_default_to_string(oe: &OptionTableEntry) -> String {
    oe.default_to_string::<COptions>()
}

pub unsafe fn options_name<'a>(o: *mut options_entry) -> &'a str {
    unsafe { (*o).name() }
}

/// The tree an entry is in.
pub unsafe fn options_owner(o: *mut options_entry) -> *mut options {
    unsafe { (*o).owner() }
}

/// The table entry of an option, null for a user option.
pub unsafe fn options_table_entry(o: *mut options_entry) -> *const options_table_entry {
    unsafe { (*o).table().map_or(null(), |oe| oe as *const options_table_entry) }
}

/// Notes that the hook in this option ran now.
pub unsafe fn options_hook_fired(o: *mut options_entry) {
    unsafe { (*o).hook_fired(CURRENT_TIME) }
}

pub unsafe fn options_array_clear(o: *mut options_entry) {
    unsafe { (*o).array_clear() }
}

/// Whether array item `key` is set.
pub unsafe fn options_array_has(o: *mut options_entry, key: &ArrayKey) -> bool {
    unsafe { (*o).array_get(key).is_some() }
}

/// A copy of string array item `key`.
pub unsafe fn options_array_string(o: *mut options_entry, key: &ArrayKey) -> Option<CString> {
    unsafe { (*o).array_get(key).and_then(Value::as_string).map(CStr::to_owned) }
}

/// Sets array item `key`, or removes it if `value` is None.
pub unsafe fn options_array_set(
    o: *mut options_entry,
    key: &ArrayKey,
    value: Option<&[u8]>,
    append: bool,
) -> Result<(), OptionsError> {
    unsafe { (*o).array_set(key, value, append) }
}

/// Adds the items of a separated list at the first free numbers.
pub unsafe fn options_array_assign(o: *mut options_entry, s: &[u8]) -> Result<(), OptionsError> {
    unsafe { (*o).array_assign(s) }
}

/// The keys of an array option, in order.
pub unsafe fn options_array_keys(o: *mut options_entry) -> Vec<ArrayKey> {
    unsafe { (*o).array_items().map(|(k, _)| k.clone()).collect() }
}

/// Copies of the items of a string array option, in key order.
pub unsafe fn options_array_strings(o: *const options_entry) -> Vec<(ArrayKey, CString)> {
    unsafe {
        (*o).array_items()
            .filter_map(|(k, v)| Some((k.clone(), v.as_string()?.to_owned())))
            .collect()
    }
}

/// The items of a number array option (colours), in key order.
pub unsafe fn options_array_numbers(o: *const options_entry) -> Vec<(ArrayKey, i64)> {
    unsafe {
        (*o).array_items()
            .filter_map(|(k, v)| Some((k.clone(), v.as_number()?)))
            .collect()
    }
}

/// The command lists of a command array option (hooks), in key order.
/// They stay valid while the option is not changed.
pub unsafe fn options_array_commands(o: *const options_entry) -> Vec<(ArrayKey, *mut cmd_list)> {
    unsafe {
        (*o).array_items()
            .filter_map(|(k, v)| Some((k.clone(), v.as_command()?.as_ptr())))
            .collect()
    }
}

pub unsafe fn options_is_array(o: *mut options_entry) -> bool {
    unsafe { (*o).is_array() }
}

pub unsafe fn options_is_string(o: *mut options_entry) -> bool {
    unsafe { (*o).is_string() }
}

/// The value as text: an array item with `key`, the whole array joined
/// with spaces, or the value. `numeric` prints flags as numbers.
pub unsafe fn options_to_string(o: *mut options_entry, key: Option<&ArrayKey>, numeric: bool) -> String {
    unsafe { (*o).to_string(key, numeric) }
}

/// Splits `name[key]` into the name and key.
pub fn options_parse(name: &str) -> Option<(String, Option<ArrayKey>)> {
    store::parse_option_name(name)
}

/// Finds an option by `name[key]` in a tree, or in it and its parents
/// without `only`. Sets `key` to the array key.
pub unsafe fn options_parse_get(
    oo: *mut options,
    s: &str,
    key: &mut Option<ArrayKey>,
    only: bool,
) -> *mut options_entry {
    unsafe {
        *key = None;
        let Some((name, parsed)) = options_parse(s) else {
            return null_mut();
        };
        let o = if only {
            options_get_only(oo, &name)
        } else {
            options_get(&mut *oo, &name)
        };
        if !o.is_null() {
            *key = parsed;
        }
        o
    }
}

/// The table entry with exactly this name.
pub fn options_search(name: &str) -> Option<&'static OptionTableEntry> {
    store::search_table(&OPTIONS_TABLE, name)
}

/// The full name and array key an option name refers to, allowing
/// unique prefixes and old names.
pub fn options_match(s: &str) -> Result<(String, Option<ArrayKey>), MatchError> {
    store::match_option_name(&OPTIONS_TABLE, s)
}

/// A string option, from the tree or its parents. The pointer stays
/// valid until the option changes. Panics if missing or not a string.
pub unsafe fn options_get_string(oo: *mut options, name: &str) -> *const u8 {
    unsafe { options_get_string_(oo, name) }
}

pub unsafe fn options_get_string_(oo: *const options, name: &str) -> *const u8 {
    unsafe {
        let Some(o) = options_get_const(oo, name) else {
            fatalx_!("missing option {name}");
        };
        if !o.is_string() {
            fatalx_!("option {name} is not a string");
        }
        o.string().map_or(c!(""), |s| s.as_ptr().cast())
    }
}

pub unsafe fn options_get_number_(oo: *const options, name: &str) -> i64 {
    unsafe {
        let Some(o) = options_get_const(oo, name) else {
            fatalx_!("missing option {name}");
        };
        match o.number() {
            Some(n) if o.is_number() => n,
            _ => fatalx_!("option {name} is not a number"),
        }
    }
}

/// A number option converted to `T`. Panics if missing, not a number or
/// out of range for `T`.
#[track_caller]
pub unsafe fn options_get_number___<T: TryFrom<i64>>(oo: &options, name: &str) -> T {
    unsafe {
        let Some(o) = options_get_const(oo, name) else {
            panic!("missing option {name}");
        };
        let Some(n) = o.number().filter(|_| o.is_number()) else {
            panic!("option {name} is not a number");
        };
        match T::try_from(n) {
            Ok(value) => value,
            Err(_) => panic!("options_get_number out of range"),
        }
    }
}

macro_rules! options_set_string {
   ($oo:expr, $name:expr, $append:expr, $fmt:literal $(, $args:expr)* $(,)?) => {
        crate::options_::options_set_string_($oo, $name, $append, format_args!($fmt $(, $args)*))
    };
}
pub(crate) use options_set_string;

/// Sets a string option from format arguments. See
/// [`options_set_string_bytes`].
pub unsafe fn options_set_string_(
    oo: *mut options,
    name: &str,
    append: bool,
    args: std::fmt::Arguments,
) -> *mut options_entry {
    unsafe { options_set_string_bytes(oo, name, args.to_string().as_bytes(), append) }
}

/// Sets a string option in this tree. With `append`, the value is added
/// to the end of the value in this tree after the option's separator.
/// Panics if the option is not a string.
pub unsafe fn options_set_string_bytes(
    oo: *mut options,
    name: &str,
    value: &[u8],
    append: bool,
) -> *mut options_entry {
    unsafe { (*oo).store.set_string(name, value, append) }
}

/// Sets a number option in this tree. Panics for a user option or an
/// option that is not a number.
pub unsafe fn options_set_number(oo: *mut options, name: &str, value: i64) -> *mut options_entry {
    unsafe { (*oo).store.set_number(name, value) }
}

/// The tree an option with this name is set in for the command's flags
/// and target, and its scope. The message is the command's error.
pub unsafe fn options_scope_from_name(
    args: *mut args,
    window: i32,
    name: &str,
    fs: *mut cmd_find_state,
    oo: *mut *mut options,
) -> Result<i32, String> {
    unsafe {
        let s = (*fs).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        let wl = (*fs).wl;
        let wp = (*fs).wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        let target = args_get_(args, 't');

        if name.starts_with('@') {
            return options_scope_from_flags(args, window, fs, oo);
        }

        let Some(oe) = OPTIONS_TABLE.iter().find(|oe| oe.name == name) else {
            return Err(format!("unknown option: {name}"));
        };

        const OPTIONS_TABLE_WINDOW_AND_PANE: i32 = OPTIONS_TABLE_WINDOW | OPTIONS_TABLE_PANE;
        let window_scope = |oo: *mut *mut options| {
            if args_has(args, 'g') {
                *oo = GLOBAL_W_OPTIONS;
                Ok(OPTIONS_TABLE_WINDOW)
            } else if wl.is_null() && !target.is_null() {
                Err(format!("no such window: {}", _s(target)))
            } else if wl.is_null() {
                Err("no current window".into())
            } else {
                *oo = (*winlink_window(wl)).options;
                Ok(OPTIONS_TABLE_WINDOW)
            }
        };
        match oe.scope {
            OPTIONS_TABLE_SERVER => {
                *oo = GLOBAL_OPTIONS;
                Ok(OPTIONS_TABLE_SERVER)
            }
            OPTIONS_TABLE_SESSION => {
                if args_has(args, 'g') {
                    *oo = GLOBAL_S_OPTIONS;
                    Ok(OPTIONS_TABLE_SESSION)
                } else if s.is_null() && !target.is_null() {
                    Err(format!("no such session: {}", _s(target)))
                } else if s.is_null() {
                    Err("no current session".into())
                } else {
                    *oo = (*s).options;
                    Ok(OPTIONS_TABLE_SESSION)
                }
            }
            OPTIONS_TABLE_WINDOW_AND_PANE if args_has(args, 'p') => {
                if wp.is_null() && !target.is_null() {
                    Err(format!("no such pane: {}", _s(target)))
                } else if wp.is_null() {
                    Err("no current pane".into())
                } else {
                    *oo = (*wp).options;
                    Ok(OPTIONS_TABLE_PANE)
                }
            }
            OPTIONS_TABLE_WINDOW_AND_PANE | OPTIONS_TABLE_WINDOW => window_scope(oo),
            _ => Ok(OPTIONS_TABLE_NONE),
        }
    }
}

/// The tree the command's flags and target select, for user options and
/// commands without an option name.
pub unsafe fn options_scope_from_flags(
    args: *mut args,
    window: i32,
    fs: *mut cmd_find_state,
    oo: *mut *mut options,
) -> Result<i32, String> {
    unsafe {
        let s = (*fs).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        let wl = (*fs).wl;
        let wp = (*fs).wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        let target = args_get_(args, 't');

        if args_has(args, 's') {
            *oo = GLOBAL_OPTIONS;
            return Ok(OPTIONS_TABLE_SERVER);
        }

        if args_has(args, 'p') {
            if wp.is_null() {
                if !target.is_null() {
                    return Err(format!("no such pane: {}", _s(target)));
                }
                return Err("no current pane".into());
            }
            *oo = (*wp).options;
            Ok(OPTIONS_TABLE_PANE)
        } else if window != 0 || args_has(args, 'w') {
            if args_has(args, 'g') {
                *oo = GLOBAL_W_OPTIONS;
                return Ok(OPTIONS_TABLE_WINDOW);
            }
            if wl.is_null() {
                if !target.is_null() {
                    return Err(format!("no such window: {}", _s(target)));
                }
                return Err("no current window".into());
            }
            *oo = (*winlink_window(wl)).options;
            Ok(OPTIONS_TABLE_WINDOW)
        } else {
            if args_has(args, 'g') {
                *oo = GLOBAL_S_OPTIONS;
                return Ok(OPTIONS_TABLE_SESSION);
            }
            if s.is_null() {
                if !target.is_null() {
                    return Err(format!("no such session: {}", _s(target)));
                }
                return Err("no current session".into());
            }
            *oo = (*s).options;
            Ok(OPTIONS_TABLE_SESSION)
        }
    }
}

/// The style in a string option, parsed and cached on the entry. A style
/// with formats is expanded with `ft` (if given) and parsed each time.
/// Null if the option is missing, not a string or does not parse.
pub unsafe fn options_string_to_style(oo: *mut options, name: &str, ft: *mut format_tree) -> *mut style {
    unsafe {
        let o = options_get(&mut *oo, name);
        if o.is_null() || !(*o).is_string() {
            return null_mut();
        }
        if let Some(sy) = (*o).cached_style() {
            return sy;
        }
        let s: CString = (*o).string().map(CStr::to_owned).unwrap_or_default();
        log_debug!("options_string_to_style: {} is '{}'", name, s.to_string_lossy());

        let mut sy: style = zeroed();
        style_set(&mut sy, &GRID_DEFAULT_CELL);
        // A style without formats can be cached, one with formats must be
        // expanded again each time. As C, the cache is kept even when the
        // style does not parse.
        let cache = !s.to_bytes().windows(2).any(|w| w == b"#{");

        let failed = if !ft.is_null() && !cache {
            let expanded = format_expand(ft, s.as_ptr().cast());
            let failed = style_parse(&mut sy, &GRID_DEFAULT_CELL, expanded) != 0;
            free_(expanded);
            failed
        } else {
            style_parse(&mut sy, &GRID_DEFAULT_CELL, s.as_ptr().cast()) != 0
        };
        let sy = (*o).store_style(sy, cache);
        if failed { null_mut() } else { sy }
    }
}

/// The index of a choice, or the "unknown value" error.
pub unsafe fn options_find_choice(oe: *const options_table_entry, value: *const u8) -> Result<i32, CString> {
    unsafe {
        (*oe)
            .find_choice(&cstr_to_string_lossy(value))
            .map(|n| n as i32)
            .map_err(|e| CString::new(e.to_string()).unwrap_or_default())
    }
}

/// Sets an option that is not an array from a string, checking the value
/// for the option's type. `oe` is null for a user option.
pub unsafe fn options_from_string(
    oo: *mut options,
    oe: *const options_table_entry,
    name: &str,
    value: Option<&[u8]>,
    append: bool,
) -> Result<(), OptionsError> {
    unsafe {
        let parents = options_chain((*oo).parent);
        (*oo).store.set_from_string(&parents, oe.as_ref(), name, value, append)
    }
}

/// Unsets an option: a global table option goes back to its default,
/// others are removed. With a key, only that array item is removed.
pub unsafe fn options_remove_or_default(
    o: *mut options_entry,
    key: Option<&ArrayKey>,
) -> Result<(), OptionsError> {
    unsafe {
        let oo = (*o).owner();
        let name = (*o).name().to_string();
        let global = oo == GLOBAL_OPTIONS || oo == GLOBAL_S_OPTIONS || oo == GLOBAL_W_OPTIONS;
        (*oo).store.remove_or_default(&name, key, global)
    }
}
