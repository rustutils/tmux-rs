// Copyright (c) 2026 tmux-rs contributors
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
//! Runs converted commands: the adapter between the command queue and
//! [`CmdCtx`].
//!
//! [`cmd_run_converted`] collects what the command may see from the queue
//! item into plain values, runs the command, then carries out the effects
//! it recorded and acts on the flow it returned. Client files are read
//! and written here, and the continuation runs from the file callback
//! with a new context.

use crate::cmd_::context::{
    ClientInfo, CmdArgs, CmdCtx, CmdCtxParts, CmdEffect, CmdFlow, CmdResult,
    EnvironChange, FileError, FilePath, FileThen, FormatSubject, PaneInfo, SessionInfo,
};
use crate::libc::{O_APPEND, O_TRUNC, strerror};
use crate::paste::{buffer_limit, now, paste_notify};
use crate::*;

/// Runs a converted command for `item`: builds a context, calls `f` in
/// it, carries out the effects and starts any file the result waits for.
/// The file callback runs continuations through here too.
///
/// # Safety
/// `item` must be a valid queue item whose command is being fired.
pub unsafe fn cmd_run_converted(
    item: *mut cmdq_item,
    f: impl FnOnce(&mut CmdCtx<'_>) -> CmdResult,
) -> cmd_retval {
    unsafe {
        let args = cmd_get_args((*item).cmd);
        // Command blocks among the arguments are printed into their
        // cache, so CmdArgs::string can return them.
        for value in &(*args).values {
            if matches!(value, args_value::Commands { .. }) {
                args_value_as_string(value);
            }
        }
        let parts = cmd_ctx_parts(item);
        // The arguments are borrowed only while the command runs. It is
        // safe code that reaches nothing but the context, so nothing
        // changes them meanwhile.
        let (result, effects) = {
            let mut ctx = CmdCtx::new(CmdArgs::new(&*args), parts);
            let result = f(&mut ctx);
            (result, ctx.take_effects())
        };
        cmd_apply_effects(item, effects);
        cmd_finish(item, result)
    }
}

/// Collects the values a context is built from: the item's clients and
/// target pane as plain values, the time and buffer-limit.
///
/// # Safety
/// `item` must be a valid queue item, and the global options must exist.
unsafe fn cmd_ctx_parts(item: *mut cmdq_item) -> CmdCtxParts {
    unsafe {
        let client_info = |c: *mut client| {
            (!c.is_null()).then(|| ClientInfo {
                id: (*c).id,
                attached: !client_get_session(c).is_null(),
                control: (*c).flags.intersects(client_flag::CONTROL),
            })
        };
        let pane_info = |id: PaneId| {
            let wp = pane_from_id(id)?;
            Some(PaneInfo {
                id,
                exited: window_pane_exited(wp),
                input_off: (*wp).flags.intersects(window_pane_flags::PANE_INPUTOFF),
                bracket_paste: (*(*wp).screen)
                    .mode
                    .intersects(mode_flag::MODE_BRACKETPASTE),
            })
        };
        let target = cmdq_get_target(item);
        CmdCtxParts {
            client: client_info(cmdq_get_client(item)),
            target_client: client_info(cmdq_get_target_client(item)),
            target_pane: (*target).wp.and_then(pane_info),
            target_session: (*target).s.and_then(|id| {
                let s = session_from_id(id)?;
                Some(SessionInfo {
                    id,
                    environ: Rc::clone(&(*s).environ),
                })
            }),
            now: now(),
            buffer_limit: buffer_limit(),
        }
    }
}

/// Shows `data` to the item's client, as `cmdq_print_data`.
///
/// # Safety
/// `item` must be a valid queue item.
unsafe fn cmd_print_bytes(item: *mut cmdq_item, data: &[u8]) {
    unsafe {
        let evb = evbuffer_new();
        if evb.is_null() {
            fatalx("out of memory");
        }
        evbuffer_add(evb, data.as_ptr().cast(), data.len());
        cmdq_print_data(item, evb);
        evbuffer_free(evb);
    }
}

/// Expands a format for the item's client and `subject`.
///
/// # Safety
/// `item` must be a valid queue item.
unsafe fn cmd_print_format(
    item: *mut cmdq_item,
    template: &[u8],
    filter: Option<&[u8]>,
    subject: FormatSubject,
) {
    unsafe {
        let ft = format_create(
            cmdq_get_client(item),
            item,
            FORMAT_NONE,
            format_flags::empty(),
        );
        match subject {
            FormatSubject::PasteBuffer(pb) => format_defaults_paste_buffer(ft, pb),
        }

        let show = match filter {
            Some(filter) => {
                let filter = CString::new(filter).unwrap_or_default();
                let expanded = format_expand(ft, filter.as_ptr().cast());
                let show = format_true(expanded);
                free_(expanded);
                show
            }
            None => true,
        };
        if show {
            let template = CString::new(template).unwrap_or_default();
            let line = format_expand(ft, template.as_ptr().cast());
            let bytes = CStr::from_ptr(line.cast()).to_bytes();
            cmd_print_bytes(item, bytes);
            free_(line);
        }

        format_free(ft);
    }
}

/// Carries out the effects a command recorded, in order.
///
/// # Safety
/// `item` must be a valid queue item.
unsafe fn cmd_apply_effects(item: *mut cmdq_item, effects: Vec<CmdEffect>) {
    unsafe {
        for effect in effects {
            match effect {
                CmdEffect::Paste(event) => paste_notify([event]),
                CmdEffect::Print(data) => cmd_print_bytes(item, &data),
                CmdEffect::PrintFormat {
                    template,
                    filter,
                    subject,
                } => cmd_print_format(item, &template, filter.as_deref(), subject),
                CmdEffect::SetSelection { client, data } => {
                    if let Some(c) = client_from_id(client)
                        && !(*c).flags.intersects(client_flag::DEAD)
                        && !client_get_session(c).is_null()
                    {
                        tty_set_selection(&raw mut (*c).tty, c!(""), data.as_ptr(), data.len());
                    }
                }
                CmdEffect::WritePane { pane, data } => {
                    if let Some(wp) = pane_from_id(pane) {
                        window_pane_write_to_pty(wp, data.as_ptr().cast(), data.len());
                    }
                }
                CmdEffect::Environ { session, change } => cmd_change_environ(item, session, change),
            }
        }
    }
}

/// Changes the global environment (`session` None) or a session's,
/// expanding a -F value for the item's target first.
///
/// # Safety
/// `item` must be a valid queue item.
unsafe fn cmd_change_environ(item: *mut cmdq_item, session: Option<SessionId>, change: EnvironChange) {
    unsafe {
        let change = match change {
            EnvironChange::Set {
                name,
                value,
                format: true,
                hidden,
            } => {
                let template = CString::new(value).unwrap_or_default();
                let expanded = format_single_from_target(item, template.as_ptr().cast());
                let value = CStr::from_ptr(expanded.cast()).to_bytes().to_vec();
                free_(expanded);
                EnvironChange::Set {
                    name,
                    value,
                    format: false,
                    hidden,
                }
            }
            other => other,
        };
        let apply = |env: &mut Environ| match change {
            EnvironChange::Set {
                name, value, hidden, ..
            } => {
                let flags = if hidden {
                    EnvironFlags::HIDDEN
                } else {
                    EnvironFlags::empty()
                };
                env.set(&name, value, flags);
            }
            EnvironChange::Clear { name } => env.clear(&name),
            EnvironChange::Unset { name } => env.unset(&name),
        };
        match session {
            None => global_environ_change(apply),
            Some(id) => {
                if let Some(s) = session_from_id(id) {
                    apply(Rc::make_mut(&mut (*s).environ));
                }
            }
        }
    }
}

/// A command waiting for a client file.
struct FileWait {
    item: *mut cmdq_item,
    then: FileThen,
}

/// Acts on a command's result: prints the error, or starts the file the
/// command waits for.
///
/// # Safety
/// `item` must be a valid queue item.
unsafe fn cmd_finish(item: *mut cmdq_item, result: CmdResult) -> cmd_retval {
    unsafe {
        let file_path = |path| match path {
            FilePath::Literal(path) => CString::new(path).unwrap_or_default(),
            FilePath::Format(format) => {
                let format = CString::new(format).unwrap_or_default();
                let expanded = format_single_from_target(item, format.as_ptr().cast());
                let path = CStr::from_ptr(expanded.cast()).to_owned();
                free_(expanded);
                path
            }
        };
        match result {
            Err(error) => {
                cmdq_error!(item, "{error}");
                cmd_retval::CMD_RETURN_ERROR
            }
            Ok(CmdFlow::Done) => cmd_retval::CMD_RETURN_NORMAL,
            Ok(CmdFlow::ReadFile { path, then }) => {
                let path = file_path(path);
                let wait = Box::into_raw(Box::new(FileWait { item, then }));
                file_read(
                    cmdq_get_client(item),
                    path.as_ptr().cast(),
                    Some(cmd_file_done),
                    wait.cast(),
                );
                cmd_retval::CMD_RETURN_WAIT
            }
            Ok(CmdFlow::WriteFile {
                path,
                append,
                data,
                then,
            }) => {
                let path = file_path(path);
                let wait = Box::into_raw(Box::new(FileWait { item, then }));
                let flags = if append { O_APPEND } else { O_TRUNC };
                file_write(
                    cmdq_get_client(item),
                    path.as_ptr().cast(),
                    flags,
                    data.as_ptr().cast(),
                    data.len(),
                    Some(cmd_file_done),
                    wait.cast(),
                );
                cmd_retval::CMD_RETURN_WAIT
            }
        }
    }
}

/// The file callback: runs the continuation once the file is closed,
/// then lets the queue go on unless it waits again.
///
/// # Safety
/// `data` is the [`FileWait`] given to `file_read` or `file_write`, and is
/// freed here once the file is closed.
unsafe fn cmd_file_done(
    _c: *mut client,
    path: *mut u8,
    error: i32,
    closed: i32,
    buffer: *mut evbuffer,
    data: *mut c_void,
) {
    if closed == 0 {
        return;
    }
    unsafe {
        let FileWait { item, then } = *Box::from_raw(data.cast::<FileWait>());
        let done = if error != 0 {
            let path = if path.is_null() {
                String::new()
            } else {
                String::from_utf8_lossy(CStr::from_ptr(path.cast()).to_bytes()).into_owned()
            };
            Err(FileError {
                message: strerror(error).to_string(),
                path,
            })
        } else if buffer.is_null() || EVBUFFER_LENGTH(buffer) == 0 {
            Ok(Vec::new())
        } else {
            let size = EVBUFFER_LENGTH(buffer);
            Ok(std::slice::from_raw_parts(EVBUFFER_DATA(buffer).cast::<u8>(), size).to_vec())
        };
        let retval = cmd_run_converted(item, |ctx| then(ctx, done));
        if retval != cmd_retval::CMD_RETURN_WAIT {
            cmdq_continue(item);
        }
    }
}
