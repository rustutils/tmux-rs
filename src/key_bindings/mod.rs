// Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
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
//! The adapter between key tables ([`store::KeyTables`]) and the rest of
//! tmux: the default bindings, table changes that move clients, and
//! running a binding.
mod store;

use std::time::{Duration, UNIX_EPOCH};

pub use self::store::{KeyBinding as StoreKeyBinding, KeyTableId, KeyTables};

use crate::global::with_key_tables;
use crate::*;

/// A binding, as the rest of tmux gets it: a copy holding its own
/// reference to the command list.
pub type KeyBinding = StoreKeyBinding<CmdList>;

/// A key table as a client or the key callback refers to it. The ID
/// finds nothing once the table is dropped, which counts as an empty
/// table, as 3.8's detached tables are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyTableRef {
    pub id: KeyTableId,
    pub name: Vec<u8>,
}

impl KeyTableRef {
    /// The table called `name`, created if it does not exist.
    pub fn get_or_create(name: &[u8]) -> Self {
        Self {
            id: with_key_tables(|t| t.get_or_create(name)),
            name: name.to_vec(),
        }
    }

    /// Whether the table is called `name`.
    pub fn is(&self, name: &[u8]) -> bool {
        self.name == name
    }

    /// The name, for logs and formats.
    pub fn name_lossy(&self) -> String {
        String::from_utf8_lossy(&self.name).into_owned()
    }
}

/// The binding of `key` (flags masked off) in table `table`.
pub fn key_bindings_get(table: &KeyTableRef, key: key_code) -> Option<KeyBinding> {
    with_key_tables(|t| t.get(table.id, key & !KEYC_MASK_FLAGS))
}

/// The binding of `key` in the table called `name`.
pub fn key_bindings_get_by_name(name: &[u8], key: key_code) -> Option<KeyBinding> {
    with_key_tables(|t| t.get_by_name(name, key & !KEYC_MASK_FLAGS))
}

/// The default binding of `key` in the table called `name`.
pub fn key_bindings_get_default(name: &[u8], key: key_code) -> Option<KeyBinding> {
    with_key_tables(|t| t.get_default(name, key & !KEYC_MASK_FLAGS))
}

/// Whether a table called `name` exists.
pub fn key_bindings_has_table(name: &[u8]) -> bool {
    with_key_tables(|t| t.id(name).is_some())
}

/// The table names in order.
pub fn key_bindings_table_names() -> Vec<Vec<u8>> {
    with_key_tables(|t| t.names().map(<[u8]>::to_vec).collect())
}

/// The bindings of the table called `name`, in key order.
pub fn key_bindings_list(name: &[u8]) -> Vec<KeyBinding> {
    with_key_tables(|t| t.bindings(name))
}

/// When a client last entered `table`, None once it was dropped.
pub fn key_table_activity(table: &KeyTableRef) -> Option<Duration> {
    with_key_tables(|t| t.activity(table.id))
}

/// Records that a client entered `table` now.
pub fn key_table_touch(table: &KeyTableRef) {
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    with_key_tables(|t| t.set_activity(table.id, now));
}

/// Binds `key` in the table called `name` (`key_bindings_add`). Without
/// `cmds`, an existing binding only gets the note and the repeat flag.
pub fn key_bindings_add(
    name: &[u8],
    key: key_code,
    note: Option<&[u8]>,
    repeat: bool,
    cmds: Option<CmdList>,
) {
    let key = key & !KEYC_MASK_FLAGS;
    if let Some(cmds) = &cmds {
        // SAFETY: the list is live while `cmds` holds it, and
        // cmd_list_print returns a new string, freed here.
        unsafe {
            let s = cmd_list_print(&*cmds.as_ptr(), 0);
            log_debug!(
                "key_bindings_add: {:#x} {} = {}",
                key,
                key_string_lookup_key(key, 1),
                _s(s),
            );
            free_(s);
        }
    }
    // The replaced binding is dropped outside the store, its command list
    // may be freed.
    let _old = with_key_tables(|t| {
        let old = t.get_by_name(name, key);
        t.add(name, key, note, repeat, cmds);
        old
    });
}

/// Unbinds `key` in the table called `name`.
pub fn key_bindings_remove(name: &[u8], key: key_code) {
    let key = key & !KEYC_MASK_FLAGS;
    let old = with_key_tables(|t| {
        let old = t.get_by_name(name, key);
        t.remove(name, key);
        old
    });
    if old.is_some() {
        log_debug!("key_bindings_remove: {:#x} {}", key, key_string_lookup_key(key, 1));
    }
}

/// Puts `key` in the table called `name` back to its default binding, or
/// unbinds it if it has none.
pub fn key_bindings_reset(name: &[u8], key: key_code) {
    let key = key & !KEYC_MASK_FLAGS;
    let _old = with_key_tables(|t| {
        let old = t.get_by_name(name, key);
        t.reset(name, key);
        old
    });
}

/// Replaces the commands of the binding of `key` in the table called
/// `name`, keeping the binding (customize mode).
pub fn key_bindings_set_commands(name: &[u8], key: key_code, cmds: CmdList) {
    // The old commands are dropped outside the store, they may be freed.
    let old = with_key_tables(|t| t.get_by_name(name, key & !KEYC_MASK_FLAGS));
    with_key_tables(|t| t.set_commands(name, key & !KEYC_MASK_FLAGS, cmds));
    drop(old);
}

/// Sets or clears the repeat flag of the binding of `key` in the table
/// called `name` (customize mode).
pub fn key_bindings_set_repeat(name: &[u8], key: key_code, repeat: bool) {
    with_key_tables(|t| t.set_repeat(name, key & !KEYC_MASK_FLAGS, repeat));
}

/// Removes the table called `name` with its defaults, and moves the
/// clients in it to their default table.
///
/// # Safety
/// The clients must be valid.
pub unsafe fn key_bindings_remove_table(name: &[u8]) {
    unsafe {
        let Some(id) = with_key_tables(|t| t.remove_table(name)) else {
            return;
        };
        for c in clients_iter() {
            if (*c).keytable.id == id {
                server_client_set_key_table(c, None);
            }
        }
    }
}

unsafe fn key_bindings_init_done(_item: *mut cmdq_item, _data: *mut c_void) -> cmd_retval {
    with_key_tables(KeyTables::snapshot_defaults);
    cmd_retval::CMD_RETURN_NORMAL
}

macro_rules! DEFAULT_SESSION_MENU {
    () => {
        concat!(
            " 'Next' 'n' {switch-client -n}",
            " 'Previous' 'p' {switch-client -p}",
            " ''",
            " 'Renumber' 'N' {move-window -r}",
            " 'Rename' 'n' {command-prompt -I \"#S\" {rename-session -- '%%'}}",
            " ''",
            " 'New Session' 's' {new-session}",
            " 'New Window' 'w' {new-window}"
        )
    };
}

macro_rules! DEFAULT_WINDOW_MENU {
    () => {
        concat!(
            " '#{?#{>:#{session_windows},1},,-}Swap Left' 'l' {swap-window -t:-1}",
            " '#{?#{>:#{session_windows},1},,-}Swap Right' 'r' {swap-window -t:+1}",
            " '#{?pane_marked_set,,-}Swap Marked' 's' {swap-window}",
            " ''",
            " 'Kill' 'X' {kill-window}",
            " 'Respawn' 'R' {respawn-window -k}",
            " '#{?pane_marked,Unmark,Mark}' 'm' {select-pane -m}",
            " 'Rename' 'n' {command-prompt -FI \"#W\" {rename-window -t '#{window_id}' -- '%%'}}",
            " ''",
            " 'New After' 'w' {new-window -a}",
            " 'New At End' 'W' {new-window}"
        )
    };
}

macro_rules! DEFAULT_PANE_MENU {
    () => {
        concat!(
            " '#{?#{m/r:(copy|view)-mode,#{pane_mode}},Go To Top,}' '<' {send -X history-top}",
            " '#{?#{m/r:(copy|view)-mode,#{pane_mode}},Go To Bottom,}' '>' {send -X history-bottom}",
            " ''",
            " '#{?mouse_word,Search For #[underscore]#{=/9/...:mouse_word},}' 'C-r' {if -F '#{?#{m/r:(copy|view)-mode,#{pane_mode}},0,1}' 'copy-mode -t='; send -Xt= search-backward \"#{q:mouse_word}\"}",
            " '#{?mouse_word,Type #[underscore]#{=/9/...:mouse_word},}' 'C-y' {copy-mode -q; send-keys -l -- \"#{q:mouse_word}\"}",
            " '#{?mouse_word,Copy #[underscore]#{=/9/...:mouse_word},}' 'c' {copy-mode -q; set-buffer -- \"#{q:mouse_word}\"}",
            " '#{?mouse_line,Copy Line,}' 'l' {copy-mode -q; set-buffer -- \"#{q:mouse_line}\"}",
            " ''",
            " '#{?mouse_hyperlink,Type #[underscore]#{=/9/...:mouse_hyperlink},}' 'C-h' {copy-mode -q; send-keys -l -- \"#{q:mouse_hyperlink}\"}",
            " '#{?mouse_hyperlink,Copy #[underscore]#{=/9/...:mouse_hyperlink},}' 'h' {copy-mode -q; set-buffer -- \"#{q:mouse_hyperlink}\"}",
            " ''",
            " 'Horizontal Split' 'h' {split-window -h}",
            " 'Vertical Split' 'v' {split-window -v}",
            " ''",
            " '#{?#{>:#{window_panes},1},,-}Swap Up' 'u' {swap-pane -U}",
            " '#{?#{>:#{window_panes},1},,-}Swap Down' 'd' {swap-pane -D}",
            " '#{?pane_marked_set,,-}Swap Marked' 's' {swap-pane}",
            " ''",
            " 'Kill' 'X' {kill-pane}",
            " 'Respawn' 'R' {respawn-pane -k}",
            " '#{?pane_marked,Unmark,Mark}' 'm' {select-pane -m}",
            " '#{?#{>:#{window_panes},1},,-}#{?window_zoomed_flag,Unzoom,Zoom}' 'z' {resize-pane -Z}",
        )
    };
}

pub unsafe fn key_bindings_init() {
    #[rustfmt::skip]
    static DEFAULTS: [&str; 264] = [
        // Prefix keys.
        "bind -N 'Send the prefix key' C-b { send-prefix }",
        "bind -N 'Rotate through the panes' C-o { rotate-window }",
        "bind -N 'Suspend the current client' C-z { suspend-client }",
        "bind -N 'Select next layout' Space { next-layout }",
        "bind -N 'Break pane to a new window' ! { break-pane }",
        "bind -N 'Split window vertically' '\"' { split-window }",
        "bind -N 'List all paste buffers' '#' { list-buffers }",
        "bind -N 'Rename current session' '$' { command-prompt -I'#S' { rename-session -- '%%' } }",
        "bind -N 'Split window horizontally' % { split-window -h }",
        "bind -N 'Kill current window' & { confirm-before -p\"kill-window #W? (y/n)\" kill-window }",
        "bind -N 'Prompt for window index to select' \"'\" { command-prompt -pindex { select-window -t ':%%' } }",
        "bind -N 'Switch to previous client' ( { switch-client -p }",
        "bind -N 'Switch to next client' ) { switch-client -n }",
        "bind -N 'Rename current window' , { command-prompt -I'#W' { rename-window -- '%%' } }",
        "bind -N 'Delete the most recent paste buffer' - { delete-buffer }",
        "bind -N 'Move the current window' . { command-prompt { move-window -t '%%' } }",
        "bind -N 'Describe key binding' '/' { command-prompt -kpkey  { list-keys -1N '%%' } }",
        "bind -N 'Select window 0' 0 { select-window -t:=0 }",
        "bind -N 'Select window 1' 1 { select-window -t:=1 }",
        "bind -N 'Select window 2' 2 { select-window -t:=2 }",
        "bind -N 'Select window 3' 3 { select-window -t:=3 }",
        "bind -N 'Select window 4' 4 { select-window -t:=4 }",
        "bind -N 'Select window 5' 5 { select-window -t:=5 }",
        "bind -N 'Select window 6' 6 { select-window -t:=6 }",
        "bind -N 'Select window 7' 7 { select-window -t:=7 }",
        "bind -N 'Select window 8' 8 { select-window -t:=8 }",
        "bind -N 'Select window 9' 9 { select-window -t:=9 }",
        "bind -N 'Prompt for a command' : { command-prompt }",
        "bind -N 'Move to the previously active pane' \\; { last-pane }",
        "bind -N 'Choose a paste buffer from a list' = { choose-buffer -Z }",
        "bind -N 'List key bindings' ? { list-keys -N }",
        "bind -N 'Choose and detach a client from a list' D { choose-client -Z }",
        "bind -N 'Spread panes out evenly' E { select-layout -E }",
        "bind -N 'Switch to the last client' L { switch-client -l }",
        "bind -N 'Clear the marked pane' M { select-pane -M }",
        "bind -N 'Change the pane title' T { command-prompt -I'#T' { select-pane -T '%%' } }",
        "bind -N 'Enter copy mode' [ { copy-mode }",
        "bind -N 'Paste the most recent paste buffer' ] { paste-buffer -p }",
        "bind -N 'Create a new window' c { new-window }",
        "bind -N 'Detach the current client' d { detach-client }",
        "bind -N 'Search for a pane' f { command-prompt { find-window -Z -- '%%' } }",
        "bind -N 'Display window information' i { display-message }",
        "bind -N 'Select the previously current window' l { last-window }",
        "bind -N 'Toggle the marked pane' m { select-pane -m }",
        "bind -N 'Select the next window' n { next-window }",
        "bind -N 'Select the next pane' o { select-pane -t:.+ }",
        "bind -N 'Customize options' C { customize-mode -Z }",
        "bind -N 'Select the previous window' p { previous-window }",
        "bind -N 'Display pane numbers' q { display-panes }",
        "bind -N 'Redraw the current client' r { refresh-client }",
        "bind -N 'Choose a session from a list' s { choose-tree -Zs }",
        "bind -N 'Show a clock' t { clock-mode }",
        "bind -N 'Choose a window from a list' w { choose-tree -Zw }",
        "bind -N 'Kill the active pane' x { confirm-before -p\"kill-pane #P? (y/n)\" kill-pane }",
        "bind -N 'Zoom the active pane' z { resize-pane -Z }",
        "bind -N 'Swap the active pane with the pane above' '{' { swap-pane -U }",
        "bind -N 'Swap the active pane with the pane below' '}' { swap-pane -D }",
        "bind -N 'Show messages' '~' { show-messages }",
        "bind -N 'Enter copy mode and scroll up' PPage { copy-mode -u }",
        "bind -N 'Select the pane above the active pane' -r Up { select-pane -U }",
        "bind -N 'Select the pane below the active pane' -r Down { select-pane -D }",
        "bind -N 'Select the pane to the left of the active pane' -r Left { select-pane -L }",
        "bind -N 'Select the pane to the right of the active pane' -r Right { select-pane -R }",
        "bind -N 'Set the even-horizontal layout' M-1 { select-layout even-horizontal }",
        "bind -N 'Set the even-vertical layout' M-2 { select-layout even-vertical }",
        "bind -N 'Set the main-horizontal layout' M-3 { select-layout main-horizontal }",
        "bind -N 'Set the main-vertical layout' M-4 { select-layout main-vertical }",
        "bind -N 'Select the tiled layout' M-5 { select-layout tiled }",
        "bind -N 'Set the main-horizontal-mirrored layout' M-6 { select-layout main-horizontal-mirrored }",
        "bind -N 'Set the main-vertical-mirrored layout' M-7 { select-layout main-vertical-mirrored }",
        "bind -N 'Select the next window with an alert' M-n { next-window -a }",
        "bind -N 'Rotate through the panes in reverse' M-o { rotate-window -D }",
        "bind -N 'Select the previous window with an alert' M-p { previous-window -a }",
        "bind -N 'Move the visible part of the window up' -r S-Up { refresh-client -U 10 }",
        "bind -N 'Move the visible part of the window down' -r S-Down { refresh-client -D 10 }",
        "bind -N 'Move the visible part of the window left' -r S-Left { refresh-client -L 10 }",
        "bind -N 'Move the visible part of the window right' -r S-Right { refresh-client -R 10 }",
        "bind -N 'Reset so the visible part of the window follows the cursor' -r DC { refresh-client -c }",
        "bind -N 'Resize the pane up by 5' -r M-Up { resize-pane -U 5 }",
        "bind -N 'Resize the pane down by 5' -r M-Down { resize-pane -D 5 }",
        "bind -N 'Resize the pane left by 5' -r M-Left { resize-pane -L 5 }",
        "bind -N 'Resize the pane right by 5' -r M-Right { resize-pane -R 5 }",
        "bind -N 'Resize the pane up' -r C-Up { resize-pane -U }",
        "bind -N 'Resize the pane down' -r C-Down { resize-pane -D }",
        "bind -N 'Resize the pane left' -r C-Left { resize-pane -L }",
        "bind -N 'Resize the pane right' -r C-Right { resize-pane -R }",
        /* Menu keys */
        concat!( "bind < { display-menu -xW -yW -T '#[align=centre]#{window_index}:#{window_name}' ", DEFAULT_WINDOW_MENU!(), " }"),
        concat!( "bind > { display-menu -xP -yP -T '#[align=centre]#{pane_index} ", "(#{pane_id})' ", DEFAULT_PANE_MENU!(), " }"),
        // Mouse button 1 down on pane.
        "bind -n MouseDown1Pane { select-pane -t=; send -M }",
        /* Mouse button 1 drag on pane. */
        "bind -n MouseDrag1Pane { if -F '#{||:#{pane_in_mode},#{mouse_any_flag}}' { send -M } { copy-mode -M } }",
        /* Mouse wheel up on pane. */
        "bind -n WheelUpPane { if -F '#{||:#{pane_in_mode},#{mouse_any_flag}}' { send -M } { copy-mode -e } }",
        /* Mouse button 2 down on pane. */
        "bind -n MouseDown2Pane { select-pane -t=; if -F '#{||:#{pane_in_mode},#{mouse_any_flag}}' { send -M } { paste -p } }",
        /* Mouse button 1 double click on pane. */
        "bind -n DoubleClick1Pane { select-pane -t=; if -F '#{||:#{pane_in_mode},#{mouse_any_flag}}' { send -M } { copy-mode -H; send -X select-word; run -d0.3; send -X copy-pipe-and-cancel } }",
        /* Mouse button 1 triple click on pane. */
        "bind -n TripleClick1Pane { select-pane -t=; if -F '#{||:#{pane_in_mode},#{mouse_any_flag}}' { send -M } { copy-mode -H; send -X select-line; run -d0.3; send -X copy-pipe-and-cancel } }",
        /* Mouse button 1 on border. */
        "bind -n MouseDown1Border { select-pane -t= }",
        /* Mouse button 1 drag on border. */
        "bind -n MouseDrag1Border { resize-pane -M }",
        /* Mouse button 1 down on status line. */
        "bind -n MouseDown1Status { switch-client -t= }",
        /* Mouse wheel down on status line. */
        "bind -n WheelDownStatus { next-window }",
        /* Mouse wheel up on status line. */
        "bind -n WheelUpStatus { previous-window }",
        /* Mouse button 3 down on status left. */
        concat!("bind -n MouseDown3StatusLeft { display-menu -t= -xM -yW -T '#[align=centre]#{session_name}' ", DEFAULT_SESSION_MENU!(), " }"),
        concat!("bind -n M-MouseDown3StatusLeft { display-menu -t= -xM -yW -T '#[align=centre]#{session_name}' ", DEFAULT_SESSION_MENU!(), " }"),
        /* Mouse button 3 down on status line. */
        concat!( "bind -n MouseDown3Status { display-menu -t= -xW -yW -T '#[align=centre]#{window_index}:#{window_name}' ", DEFAULT_WINDOW_MENU!(), "}"),
        concat!( "bind -n M-MouseDown3Status { display-menu -t= -xW -yW -T '#[align=centre]#{window_index}:#{window_name}' ", DEFAULT_WINDOW_MENU!(), "}"),
        /* Mouse button 3 down on pane. */
        concat!( "bind -n MouseDown3Pane { if -Ft= '#{||:#{mouse_any_flag},#{&&:#{pane_in_mode},#{?#{m/r:(copy|view)-mode,#{pane_mode}},0,1}}}' { select-pane -t=; send -M } { display-menu -t= -xM -yM -T '#[align=centre]#{pane_index} (#{pane_id})' ", DEFAULT_PANE_MENU!(), " } }"),
        concat!( "bind -n M-MouseDown3Pane { display-menu -t= -xM -yM -T '#[align=centre]#{pane_index} (#{pane_id})' ", DEFAULT_PANE_MENU!(), " }"),
        /* Copy mode (emacs) keys. */
        "bind -Tcopy-mode C-Space { send -X begin-selection }",
        "bind -Tcopy-mode C-a { send -X start-of-line }",
        "bind -Tcopy-mode C-c { send -X cancel }",
        "bind -Tcopy-mode C-e { send -X end-of-line }",
        "bind -Tcopy-mode C-f { send -X cursor-right }",
        "bind -Tcopy-mode C-b { send -X cursor-left }",
        "bind -Tcopy-mode C-g { send -X clear-selection }",
        "bind -Tcopy-mode C-k { send -X copy-pipe-end-of-line-and-cancel }",
        "bind -Tcopy-mode C-n { send -X cursor-down }",
        "bind -Tcopy-mode C-p { send -X cursor-up }",
        "bind -Tcopy-mode C-r { command-prompt -P -T search -ip'(search up)' -I'#{pane_search_string}' { send -X search-backward-incremental '%%' } }",
        "bind -Tcopy-mode C-s { command-prompt -P -T search -ip'(search down)' -I'#{pane_search_string}' { send -X search-forward-incremental '%%' } }",
        "bind -Tcopy-mode C-v { send -X page-down }",
        "bind -Tcopy-mode C-w { send -X copy-pipe-and-cancel }",
        "bind -Tcopy-mode Escape { send -X cancel }",
        "bind -Tcopy-mode Space { send -X page-down }",
        "bind -Tcopy-mode , { send -X jump-reverse }",
        "bind -Tcopy-mode \\; { send -X jump-again }",
        "bind -Tcopy-mode F { command-prompt -P -1p'(jump backward)' { send -X jump-backward '%%' } }",
        "bind -Tcopy-mode N { send -X search-reverse }",
        "bind -Tcopy-mode P { send -X toggle-position }",
        "bind -Tcopy-mode R { send -X rectangle-toggle }",
        "bind -Tcopy-mode T { command-prompt -P -1p'(jump to backward)' { send -X jump-to-backward '%%' } }",
        "bind -Tcopy-mode X { send -X set-mark }",
        "bind -Tcopy-mode f { command-prompt -P -1p'(jump forward)' { send -X jump-forward '%%' } }",
        "bind -Tcopy-mode g { command-prompt -P -p'(goto line)' { send -X goto-line '%%' } }",
        "bind -Tcopy-mode n { send -X search-again }",
        "bind -Tcopy-mode q { send -X cancel }",
        "bind -Tcopy-mode r { send -X refresh-from-pane }",
        "bind -Tcopy-mode t { command-prompt -P -1p'(jump to forward)' { send -X jump-to-forward '%%' } }",
        "bind -Tcopy-mode Home { send -X start-of-line }",
        "bind -Tcopy-mode End { send -X end-of-line }",
        "bind -Tcopy-mode MouseDown1Pane select-pane",
        "bind -Tcopy-mode MouseDrag1Pane { select-pane; send -X begin-selection }",
        "bind -Tcopy-mode MouseDragEnd1Pane { send -X copy-pipe-and-cancel }",
        "bind -Tcopy-mode WheelUpPane { select-pane; send -N5 -X scroll-up }",
        "bind -Tcopy-mode WheelDownPane { select-pane; send -N5 -X scroll-down }",
        "bind -Tcopy-mode DoubleClick1Pane { select-pane; send -X select-word; run -d0.3; send -X copy-pipe-and-cancel }",
        "bind -Tcopy-mode TripleClick1Pane { select-pane; send -X select-line; run -d0.3; send -X copy-pipe-and-cancel }",
        "bind -Tcopy-mode NPage { send -X page-down }",
        "bind -Tcopy-mode PPage { send -X page-up }",
        "bind -Tcopy-mode Up { send -X cursor-up }",
        "bind -Tcopy-mode Down { send -X cursor-down }",
        "bind -Tcopy-mode Left { send -X cursor-left }",
        "bind -Tcopy-mode Right { send -X cursor-right }",
        "bind -Tcopy-mode M-1 { command-prompt -P -Np'(repeat)' -I1 { send -N '%%' } }",
        "bind -Tcopy-mode M-2 { command-prompt -P -Np'(repeat)' -I2 { send -N '%%' } }",
        "bind -Tcopy-mode M-3 { command-prompt -P -Np'(repeat)' -I3 { send -N '%%' } }",
        "bind -Tcopy-mode M-4 { command-prompt -P -Np'(repeat)' -I4 { send -N '%%' } }",
        "bind -Tcopy-mode M-5 { command-prompt -P -Np'(repeat)' -I5 { send -N '%%' } }",
        "bind -Tcopy-mode M-6 { command-prompt -P -Np'(repeat)' -I6 { send -N '%%' } }",
        "bind -Tcopy-mode M-7 { command-prompt -P -Np'(repeat)' -I7 { send -N '%%' } }",
        "bind -Tcopy-mode M-8 { command-prompt -P -Np'(repeat)' -I8 { send -N '%%' } }",
        "bind -Tcopy-mode M-9 { command-prompt -P -Np'(repeat)' -I9 { send -N '%%' } }",
        "bind -Tcopy-mode M-< { send -X history-top }",
        "bind -Tcopy-mode M-> { send -X history-bottom }",
        "bind -Tcopy-mode M-R { send -X top-line }",
        "bind -Tcopy-mode M-b { send -X previous-word }",
        "bind -Tcopy-mode C-M-b { send -X previous-matching-bracket }",
        "bind -Tcopy-mode M-f { send -X next-word-end }",
        "bind -Tcopy-mode C-M-f { send -X next-matching-bracket }",
        "bind -Tcopy-mode M-m { send -X back-to-indentation }",
        "bind -Tcopy-mode M-r { send -X middle-line }",
        "bind -Tcopy-mode M-v { send -X page-up }",
        "bind -Tcopy-mode M-w { send -X copy-pipe-and-cancel }",
        "bind -Tcopy-mode M-x { send -X jump-to-mark }",
        "bind -Tcopy-mode 'M-{' { send -X previous-paragraph }",
        "bind -Tcopy-mode 'M-}' { send -X next-paragraph }",
        "bind -Tcopy-mode M-Up { send -X halfpage-up }",
        "bind -Tcopy-mode M-Down { send -X halfpage-down }",
        "bind -Tcopy-mode C-Up { send -X scroll-up }",
        "bind -Tcopy-mode C-Down { send -X scroll-down }",
        /* Copy mode (vi) keys. */
        "bind -Tcopy-mode-vi '#' { send -FX search-backward '#{copy_cursor_word}' }",
        "bind -Tcopy-mode-vi * { send -FX search-forward '#{copy_cursor_word}' }",
        "bind -Tcopy-mode-vi C-c { send -X cancel }",
        "bind -Tcopy-mode-vi C-d { send -X halfpage-down }",
        "bind -Tcopy-mode-vi C-e { send -X scroll-down }",
        "bind -Tcopy-mode-vi C-b { send -X page-up }",
        "bind -Tcopy-mode-vi C-f { send -X page-down }",
        "bind -Tcopy-mode-vi C-h { send -X cursor-left }",
        "bind -Tcopy-mode-vi C-j { send -X copy-pipe-and-cancel }",
        "bind -Tcopy-mode-vi Enter { send -X copy-pipe-and-cancel }",
        "bind -Tcopy-mode-vi C-u { send -X halfpage-up }",
        "bind -Tcopy-mode-vi C-v { send -X rectangle-toggle }",
        "bind -Tcopy-mode-vi C-y { send -X scroll-up }",
        "bind -Tcopy-mode-vi Escape { send -X clear-selection }",
        "bind -Tcopy-mode-vi Space { send -X begin-selection }",
        "bind -Tcopy-mode-vi '$' { send -X end-of-line }",
        "bind -Tcopy-mode-vi , { send -X jump-reverse }",
        "bind -Tcopy-mode-vi / { command-prompt -P -T search -p'(search down)' { send -X search-forward '%%' } }",
        "bind -Tcopy-mode-vi 0 { send -X start-of-line }",
        "bind -Tcopy-mode-vi 1 { command-prompt -P -Np'(repeat)' -I1 { send -N '%%' } }",
        "bind -Tcopy-mode-vi 2 { command-prompt -P -Np'(repeat)' -I2 { send -N '%%' } }",
        "bind -Tcopy-mode-vi 3 { command-prompt -P -Np'(repeat)' -I3 { send -N '%%' } }",
        "bind -Tcopy-mode-vi 4 { command-prompt -P -Np'(repeat)' -I4 { send -N '%%' } }",
        "bind -Tcopy-mode-vi 5 { command-prompt -P -Np'(repeat)' -I5 { send -N '%%' } }",
        "bind -Tcopy-mode-vi 6 { command-prompt -P -Np'(repeat)' -I6 { send -N '%%' } }",
        "bind -Tcopy-mode-vi 7 { command-prompt -P -Np'(repeat)' -I7 { send -N '%%' } }",
        "bind -Tcopy-mode-vi 8 { command-prompt -P -Np'(repeat)' -I8 { send -N '%%' } }",
        "bind -Tcopy-mode-vi 9 { command-prompt -P -Np'(repeat)' -I9 { send -N '%%' } }",
        "bind -Tcopy-mode-vi : { command-prompt -P -p'(goto line)' { send -X goto-line '%%' } }",
        "bind -Tcopy-mode-vi \\; { send -X jump-again }",
        "bind -Tcopy-mode-vi ? { command-prompt -P -T search -p'(search up)' { send -X search-backward '%%' } }",
        "bind -Tcopy-mode-vi A { send -X append-selection-and-cancel }",
        "bind -Tcopy-mode-vi B { send -X previous-space }",
        "bind -Tcopy-mode-vi D { send -X copy-pipe-end-of-line-and-cancel }",
        "bind -Tcopy-mode-vi E { send -X next-space-end }",
        "bind -Tcopy-mode-vi F { command-prompt -P -1p'(jump backward)' { send -X jump-backward '%%' } }",
        "bind -Tcopy-mode-vi G { send -X history-bottom }",
        "bind -Tcopy-mode-vi H { send -X top-line }",
        "bind -Tcopy-mode-vi J { send -X scroll-down }",
        "bind -Tcopy-mode-vi K { send -X scroll-up }",
        "bind -Tcopy-mode-vi L { send -X bottom-line }",
        "bind -Tcopy-mode-vi M { send -X middle-line }",
        "bind -Tcopy-mode-vi N { send -X search-reverse }",
        "bind -Tcopy-mode-vi P { send -X toggle-position }",
        "bind -Tcopy-mode-vi T { command-prompt -P -1p'(jump to backward)' { send -X jump-to-backward '%%' } }",
        "bind -Tcopy-mode-vi V { send -X select-line }",
        "bind -Tcopy-mode-vi W { send -X next-space }",
        "bind -Tcopy-mode-vi X { send -X set-mark }",
        "bind -Tcopy-mode-vi ^ { send -X back-to-indentation }",
        "bind -Tcopy-mode-vi b { send -X previous-word }",
        "bind -Tcopy-mode-vi e { send -X next-word-end }",
        "bind -Tcopy-mode-vi f { command-prompt -P -1p'(jump forward)' { send -X jump-forward '%%' } }",
        "bind -Tcopy-mode-vi g { send -X history-top }",
        "bind -Tcopy-mode-vi h { send -X cursor-left }",
        "bind -Tcopy-mode-vi j { send -X cursor-down }",
        "bind -Tcopy-mode-vi k { send -X cursor-up }",
        "bind -Tcopy-mode-vi z { send -X scroll-middle }",
        "bind -Tcopy-mode-vi l { send -X cursor-right }",
        "bind -Tcopy-mode-vi n { send -X search-again }",
        "bind -Tcopy-mode-vi o { send -X other-end }",
        "bind -Tcopy-mode-vi q { send -X cancel }",
        "bind -Tcopy-mode-vi r { send -X refresh-from-pane }",
        "bind -Tcopy-mode-vi t { command-prompt -P -1p'(jump to forward)' { send -X jump-to-forward '%%' } }",
        "bind -Tcopy-mode-vi v { send -X rectangle-toggle }",
        "bind -Tcopy-mode-vi w { send -X next-word }",
        "bind -Tcopy-mode-vi '{' { send -X previous-paragraph }",
        "bind -Tcopy-mode-vi '}' { send -X next-paragraph }",
        "bind -Tcopy-mode-vi % { send -X next-matching-bracket }",
        "bind -Tcopy-mode-vi Home { send -X start-of-line }",
        "bind -Tcopy-mode-vi End { send -X end-of-line }",
        "bind -Tcopy-mode-vi MouseDown1Pane { select-pane }",
        "bind -Tcopy-mode-vi MouseDrag1Pane { select-pane; send -X begin-selection }",
        "bind -Tcopy-mode-vi MouseDragEnd1Pane { send -X copy-pipe-and-cancel }",
        "bind -Tcopy-mode-vi WheelUpPane { select-pane; send -N5 -X scroll-up }",
        "bind -Tcopy-mode-vi WheelDownPane { select-pane; send -N5 -X scroll-down }",
        "bind -Tcopy-mode-vi DoubleClick1Pane { select-pane; send -X select-word; run -d0.3; send -X copy-pipe-and-cancel }",
        "bind -Tcopy-mode-vi TripleClick1Pane { select-pane; send -X select-line; run -d0.3; send -X copy-pipe-and-cancel }",
        "bind -Tcopy-mode-vi BSpace { send -X cursor-left }",
        "bind -Tcopy-mode-vi NPage { send -X page-down }",
        "bind -Tcopy-mode-vi PPage { send -X page-up }",
        "bind -Tcopy-mode-vi Up { send -X cursor-up }",
        "bind -Tcopy-mode-vi Down { send -X cursor-down }",
        "bind -Tcopy-mode-vi Left { send -X cursor-left }",
        "bind -Tcopy-mode-vi Right { send -X cursor-right }",
        "bind -Tcopy-mode-vi M-x { send -X jump-to-mark }",
        "bind -Tcopy-mode-vi C-Up { send -X scroll-up }",
        "bind -Tcopy-mode-vi C-Down { send -X scroll-down }",
    ];

    unsafe {
        for default in DEFAULTS {
            match cmd_parse_from_string(default, None) {
                Err(error) => {
                    log_debug!("{}", error.to_string_lossy());
                    fatalx_!("bad default key: {}", default);
                }
                Ok(cmdlist) => {
                    cmdq_append(null_mut(), cmdq_get_command(cmdlist, null_mut()));
                    cmd_list_free(cmdlist);
                }
            }
        }
        cmdq_append(
            null_mut(),
            cmdq_get_callback!(key_bindings_init_done, null_mut()).as_ptr(),
        );
    }
}

pub unsafe fn key_bindings_read_only(item: *mut cmdq_item, _data: *mut c_void) -> cmd_retval {
    unsafe {
        cmdq_error!(item, "client is read-only");
    }
    cmd_retval::CMD_RETURN_ERROR
}

pub unsafe fn key_bindings_dispatch(
    bd: &KeyBinding,
    item: *mut cmdq_item,
    c: *mut client,
    event: *mut key_event,
    fs: *mut cmd_find_state,
) -> *mut cmdq_item {
    unsafe {
        let mut flags = cmdq_state_flags::empty();

        let readonly = if c.is_null() || !(*c).flags.intersects(client_flag::READONLY) {
            true
        } else {
            cmd_list_all_have(bd.cmds.as_ptr(), cmd_flag::CMD_READONLY)
        };

        let mut new_item;
        if !readonly {
            new_item = cmdq_get_callback!(key_bindings_read_only, null_mut()).as_ptr();
        } else {
            if bd.repeat {
                flags |= cmdq_state_flags::CMDQ_STATE_REPEAT;
            }
            let new_state = cmdq_new_state(fs, event, flags);
            new_item = cmdq_get_command(bd.cmds.as_ptr(), new_state);
            cmdq_free_state(new_state);
        }
        if !item.is_null() {
            new_item = cmdq_insert_after(item, new_item);
        } else {
            new_item = cmdq_append(c, new_item);
        }
        new_item
    }
}
