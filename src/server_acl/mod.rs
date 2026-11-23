// Copyright (c) 2021 Holland Schutte, Jayson Morberg
// Copyright (c) 2021 Dallas Lyons <dallasdlyons@gmail.com>
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

//! The server access control list for the rest of tmux.
//!
//! [`ServerAcl`] in [`store`] decides who may connect and is safe Rust.
//! The functions here are the adapter: they reach it through the bridge
//! in `global.rs`, and when it reports a change they check every client
//! again, making clients that lost access exit and setting the read only
//! flag of the others. Those are unsafe until clients are converted.

pub mod store;

use std::ffi::CStr;

pub use store::{Access, AclEvent, AclId, ServerAcl};

use crate::global::with_acl;
use crate::{
    client, client_flag, clients_iter, cmdq_item, cmdq_print, proc_get_peer_gid, proc_get_peer_uid,
};

/// The user and group ID of a client's peer, None where they could not
/// be read.
///
/// # Safety
/// `c` must be a valid client with a valid peer.
unsafe fn server_acl_peer_ids(c: *mut client) -> (Option<u32>, Option<u32>) {
    unsafe {
        let uid = proc_get_peer_uid((*c).peer);
        let gid = proc_get_peer_gid((*c).peer);
        (
            (uid != libc::uid_t::MAX).then_some(uid),
            (gid != libc::gid_t::MAX).then_some(gid),
        )
    }
}

/// Applies an access check result to a client: no access makes it exit,
/// otherwise its read only flag follows the access.
///
/// # Safety
/// `c` must be a valid client.
unsafe fn server_acl_apply(c: *mut client, access: Option<Access>) {
    unsafe {
        match access {
            None => {
                (*c).exit_message = Some("access not allowed".to_string());
                (*c).flags |= client_flag::EXIT;
            }
            Some(Access::ReadOnly) => (*c).flags |= client_flag::READONLY,
            Some(Access::ReadWrite) => (*c).flags &= !client_flag::READONLY,
        }
    }
}

/// Checks every client against the list again, after a change.
///
/// # Safety
/// The client registry must hold valid clients.
unsafe fn server_acl_update(event: Option<AclEvent>) {
    let Some(AclEvent::Changed) = event else {
        return;
    };
    unsafe {
        let clients: Vec<(*mut client, Option<u32>, Option<u32>)> = clients_iter()
            .map(|c| {
                let (uid, gid) = server_acl_peer_ids(c);
                (c, uid, gid)
            })
            .collect();
        let access: Vec<Option<Access>> = with_acl(|acl| {
            clients
                .iter()
                .map(|&(_, uid, gid)| acl.check(uid, gid))
                .collect()
        });
        for (&(c, _, _), access) in clients.iter().zip(access) {
            server_acl_apply(c, access);
        }
    }
}

/// Sets up the list for a new server: root and the user running it.
pub fn server_acl_init() {
    // SAFETY: getuid has no preconditions and cannot fail.
    let owner = unsafe { libc::getuid() };
    with_acl(|acl| *acl = ServerAcl::new(owner));
}

/// Whether there is an entry for `id`.
pub fn server_acl_find(id: AclId) -> bool {
    with_acl(|acl| acl.contains(id))
}

/// Adds an entry with write access, if there is none.
pub fn server_acl_allow(id: AclId) {
    with_acl(|acl| acl.allow(id));
}

/// Removes an entry and makes clients that no longer match exit.
///
/// # Safety
/// The client registry must hold valid clients.
pub unsafe fn server_acl_deny(id: AclId) {
    let event = with_acl(|acl| acl.deny(id));
    // SAFETY: the caller guarantees the clients are valid.
    unsafe { server_acl_update(event) };
}

/// Gives an entry write access and updates the clients.
///
/// # Safety
/// The client registry must hold valid clients.
pub unsafe fn server_acl_allow_write(id: AclId) {
    let event = with_acl(|acl| acl.allow_write(id));
    // SAFETY: the caller guarantees the clients are valid.
    unsafe { server_acl_update(event) };
}

/// Takes write access from an entry and updates the clients.
///
/// # Safety
/// The client registry must hold valid clients.
pub unsafe fn server_acl_deny_write(id: AclId) {
    let event = with_acl(|acl| acl.deny_write(id));
    // SAFETY: the caller guarantees the clients are valid.
    unsafe { server_acl_update(event) };
}

/// The name of a user or group, or "unknown".
///
/// # Safety
/// Calls getpwuid and getgrgid, whose results are only valid until the
/// next call. The name is copied before returning.
unsafe fn server_acl_name(id: AclId) -> String {
    unsafe {
        let name = match id {
            AclId::User(uid) => {
                let pw = libc::getpwuid(uid);
                if pw.is_null() { None } else { Some((*pw).pw_name) }
            }
            AclId::Group(gid) => {
                let gr = libc::getgrgid(gid);
                if gr.is_null() { None } else { Some((*gr).gr_name) }
            }
        };
        match name {
            Some(name) => CStr::from_ptr(name).to_string_lossy().into_owned(),
            None => "unknown".to_string(),
        }
    }
}

/// Prints the list for server-access -l, as "name (U,W)" with U or G for
/// user or group and R or W for the access. Root is left out.
///
/// # Safety
/// `item` must be a valid command queue item.
pub unsafe fn server_acl_display(item: *mut cmdq_item) {
    let entries: Vec<(AclId, bool)> = with_acl(|acl| acl.iter().collect());
    unsafe {
        for (id, read_only) in entries {
            if id == AclId::User(0) {
                continue;
            }
            let kind = if matches!(id, AclId::User(_)) { 'U' } else { 'G' };
            let access = if read_only { 'R' } else { 'W' };
            cmdq_print!(item, "{} ({kind},{access})", server_acl_name(id));
        }
    }
}

/// Checks a new client. Returns false if it is not allowed, otherwise
/// sets its read only flag if its entry says so.
///
/// # Safety
/// `c` must be a valid client with a valid peer.
pub unsafe fn server_acl_join(c: *mut client) -> bool {
    unsafe {
        let (uid, gid) = server_acl_peer_ids(c);
        let Some(access) = with_acl(|acl| acl.check(uid, gid)) else {
            return false;
        };
        if access == Access::ReadOnly {
            (*c).flags |= client_flag::READONLY;
        }
        true
    }
}
