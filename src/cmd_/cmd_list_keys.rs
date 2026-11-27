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
use crate::*;
use crate::sort::SortCriteria;
use crate::options_::options_get_number___;

/// The default list-keys format: a bind-key command per binding, or with
/// -N the key and its note.
const LIST_KEYS_TEMPLATE: &str = concat!(
    "#{?notes_only,",
    "#{key_prefix} ",
    "#{p|#{key_string_width}:key_string} ",
    "#{?key_note,#{key_note},#{key_command}}",
    ",",
    "bind-key #{?key_has_repeat,#{?key_repeat,-r,  },} ",
    "-T #{p|#{key_table_width}:key_table} ",
    "#{p|#{key_string_width}:#{q|a:key_string}} ",
    "#{key_command}}"
);

pub static CMD_LIST_KEYS_ENTRY: cmd_entry = cmd_entry {
    name: "list-keys",
    alias: Some("lsk"),

    args: args_parse::new("1aF:NO:P:rT:", 0, 1, None),
    usage: "[-1aNr] [-F format] [-O order] [-P prefix-string][-T key-table] [key]",

    flags: cmd_flag::CMD_STARTSERVER.union(cmd_flag::CMD_AFTERHOOK),
    exec: CmdExec::Old(cmd_list_keys_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

pub static CMD_LIST_COMMANDS_ENTRY: cmd_entry = cmd_entry {
    name: "list-commands",
    alias: Some("lscm"),

    args: args_parse::new("F:", 0, 1, None),
    usage: "[-F format] [command]",

    flags: cmd_flag::CMD_STARTSERVER.union(cmd_flag::CMD_AFTERHOOK),
    exec: CmdExec::Old(cmd_list_keys_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

/// The prefix shown with -N: the -P argument, or the prefix key.
///
/// # Safety
/// `args` must be valid parsed arguments.
unsafe fn cmd_list_keys_get_prefix(args: *mut args) -> String {
    unsafe {
        if args_has(args, 'P') {
            return cstr_to_str(args_get(args, b'P')).to_string();
        }
        let prefix: key_code = options_get_number___::<i64>(&*GLOBAL_S_OPTIONS, "prefix") as _;
        if prefix == KEYC_NONE {
            String::new()
        } else {
            key_string_lookup_key(prefix, 0)
        }
    }
}

/// The bindings of the root and prefix tables, for -N without -T. Each
/// table is sorted on its own.
fn cmd_list_keys_get_root_and_prefix(criteria: &SortCriteria) -> Vec<TableBinding> {
    let mut l = Vec::new();
    for name in [b"prefix".as_slice(), b"root"] {
        l.extend(sort_get_key_bindings_table(name, criteria));
    }
    l
}

/// Adds the variables of one binding to the format tree.
///
/// # Safety
/// `ft` must be a valid format tree and `tb` a valid binding.
unsafe fn cmd_list_keys_format_add_key_binding(ft: *mut format_tree, tb: &TableBinding, prefix: &str) {
    unsafe {
        let bd = &tb.bd;
        format_add!(ft, "key_repeat", "{}", i32::from(bd.repeat));
        format_add!(
            ft,
            "key_note",
            "{}",
            String::from_utf8_lossy(bd.note.as_deref().unwrap_or_default())
        );
        format_add!(ft, "key_prefix", "{prefix}");
        format_add_bytes(ft, b"key_table", &tb.table);
        format_add!(ft, "key_string", "{}", key_string_lookup_key(bd.key, 0));

        let s = cmd_list_print(&*bd.cmds.as_ptr(), CMD_LIST_PRINT_ESCAPED | CMD_LIST_PRINT_NO_GROUPS);
        format_add!(ft, "key_command", "{}", _s(s));
        free_(s);
    }
}

/// Lists key bindings through a format, sorted by -O and -r.
unsafe fn cmd_list_keys_exec(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        if std::ptr::eq(cmd_get_entry(self_), &CMD_LIST_COMMANDS_ENTRY) {
            return cmd_list_keys_commands(self_, item);
        }

        let args = cmd_get_args(self_);
        let tc = cmdq_get_target_client(item);

        let keystr = args_string(args, 0);
        let mut only: key_code = KEYC_UNKNOWN;
        if !keystr.is_null() {
            only = key_string_lookup_string(keystr);
            if only == KEYC_UNKNOWN {
                cmdq_error!(item, "invalid key: {}", _s(keystr));
                return cmd_retval::CMD_RETURN_ERROR;
            }
            only &= KEYC_MASK_KEY | KEYC_MASK_MODIFIERS;
        }

        let criteria = match args_sort_criteria(args) {
            Ok(criteria) => criteria,
            Err(e) => {
                cmdq_error!(item, "{e}");
                return cmd_retval::CMD_RETURN_ERROR;
            }
        };

        let tablename = args_get(args, b'T');
        let mut table: Option<&[u8]> = None;
        if !tablename.is_null() {
            let name = CStr::from_ptr(tablename.cast()).to_bytes();
            if !key_bindings_has_table(name) {
                cmdq_error!(item, "table {} doesn't exist", _s(tablename));
                return cmd_retval::CMD_RETURN_ERROR;
            }
            table = Some(name);
        }

        let prefix = cmd_list_keys_get_prefix(args);
        let single = args_has(args, '1');
        let notes_only = args_has(args, 'N');

        let mut template = args_get(args, b'F');
        let default_template;
        if template.is_null() {
            default_template = CString::new(LIST_KEYS_TEMPLATE).unwrap();
            template = default_template.as_ptr().cast();
        }

        let mut l = if let Some(table) = table {
            sort_get_key_bindings_table(table, &criteria)
        } else if notes_only {
            cmd_list_keys_get_root_and_prefix(&criteria)
        } else {
            sort_get_key_bindings(&criteria)
        };

        let filter_notes = notes_only && !args_has(args, 'a');
        let filter_key = only != KEYC_UNKNOWN;
        l.retain(|tb| {
            let key = tb.bd.key & (KEYC_MASK_KEY | KEYC_MASK_MODIFIERS);
            !(filter_key && key != only) && !(filter_notes && tb.bd.note.is_none())
        });
        if filter_key && l.is_empty() {
            cmdq_error!(item, "unknown key: {}", _s(keystr));
            return cmd_retval::CMD_RETURN_ERROR;
        }
        if single {
            l.truncate(1);
        }

        let has_repeat = l.iter().any(|tb| tb.bd.repeat);
        let key_width = l
            .iter()
            .map(|tb| utf8_cstrwidth_(key_string_lookup_key(tb.bd.key, 0).as_bytes()))
            .max()
            .unwrap_or(0);
        let table_width = l
            .iter()
            .map(|tb| utf8_cstrwidth_(&tb.table))
            .max()
            .unwrap_or(0);

        let ft = format_create(cmdq_get_client(item), item, FORMAT_NONE, format_flags::empty());
        format_defaults(ft, tc, None, None, None);
        format_add!(ft, "notes_only", "{}", i32::from(notes_only));
        format_add!(ft, "key_has_repeat", "{}", i32::from(has_repeat));
        format_add!(ft, "key_string_width", "{key_width}");
        format_add!(ft, "key_table_width", "{table_width}");
        for tb in l {
            cmd_list_keys_format_add_key_binding(ft, &tb, &prefix);

            let line = format_expand(ft, template);
            if single && !tc.is_null() && !(*tc).flags.intersects(client_flag::CONTROL) {
                status_message_set!(tc, -1, 1, false, "{}", cstr_to_string_lossy(line));
            } else if *line != b'\0' {
                cmdq_print_bytes(item, CStr::from_ptr(line.cast()).to_bytes());
            }
            free_(line);

            if single {
                break;
            }
        }
        format_free(ft);

        cmd_retval::CMD_RETURN_NORMAL
    }
}

unsafe fn cmd_list_keys_commands(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        let args = cmd_get_args(self_);

        let mut template = args_get_(args, 'F');
        if template.is_null() {
            template = cstring_concat!(
                "#{command_list_name}",
                "#{?command_list_alias, (#{command_list_alias}),} ",
                "#{command_list_usage}"
            )
            .as_ptr()
            .cast();
        }

        let ft = format_create(
            cmdq_get_client(item),
            item,
            FORMAT_NONE,
            format_flags::empty(),
        );
        format_defaults(ft, null_mut(), None, None, None);

        let command = args_string(args, 0);

        for entry in CMD_TABLE {
            if !command.is_null()
                && (!streq_(command, entry.name)
                    && entry.alias.is_none_or(|alias| !streq_(command, alias)))
            {
                continue;
            }

            format_add!(ft, "command_list_name", "{}", entry.name);
            format_add!(
                ft,
                "command_list_alias",
                "{}",
                entry.alias.unwrap_or_default()
            );
            format_add!(ft, "command_list_usage", "{}", entry.usage);

            let line = format_expand(ft, template);
            if *line != b'\0' {
                cmdq_print_bytes(item, CStr::from_ptr(line.cast()).to_bytes());
            }
            free_(line);
        }

        format_free(ft);
        cmd_retval::CMD_RETURN_NORMAL
    }
}
