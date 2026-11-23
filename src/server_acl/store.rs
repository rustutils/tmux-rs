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
#![forbid(unsafe_code)]
//! The server access control list: which users and groups may connect,
//! and whether they get write access.
//!
//! A client is matched by its user ID first and by its group ID only when
//! its user has no entry. A client that matches nothing is not allowed.
//! Changing an entry returns [`AclEvent::Changed`], which tells the
//! caller to apply [`ServerAcl::check`] to every connected client again.

use std::collections::BTreeMap;

/// What an entry is for. Users sort before groups, then by ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AclId {
    /// A user ID.
    User(u32),
    /// A group ID.
    Group(u32),
}

/// What a client may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Read only, the client cannot change anything.
    ReadOnly,
    /// Read and write.
    ReadWrite,
}

/// Something the caller has to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AclEvent {
    /// An entry was removed or its write access changed. Check every
    /// client again: one that is not allowed must exit, the others get
    /// the read only flag their access says.
    Changed,
}

/// The access control list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerAcl {
    /// Entries and whether they are read only.
    entries: BTreeMap<AclId, bool>,
}

impl ServerAcl {
    /// The list a server starts with: root and the user running the
    /// server, both with write access.
    pub fn new(owner: u32) -> Self {
        let mut acl = Self::default();
        acl.allow(AclId::User(0));
        acl.allow(AclId::User(owner));
        acl
    }

    /// Whether there is an entry for `id`.
    pub fn contains(&self, id: AclId) -> bool {
        self.entries.contains_key(&id)
    }

    /// Adds an entry with write access. Returns false if there already is
    /// one, which is left as it is.
    pub fn allow(&mut self, id: AclId) -> bool {
        if self.contains(id) {
            return false;
        }
        self.entries.insert(id, false);
        true
    }

    /// Removes an entry. None if there was none.
    pub fn deny(&mut self, id: AclId) -> Option<AclEvent> {
        self.entries.remove(&id).map(|_| AclEvent::Changed)
    }

    /// Gives an entry write access. None if there is no entry.
    pub fn allow_write(&mut self, id: AclId) -> Option<AclEvent> {
        self.set_read_only(id, false)
    }

    /// Takes write access from an entry. None if there is no entry.
    pub fn deny_write(&mut self, id: AclId) -> Option<AclEvent> {
        self.set_read_only(id, true)
    }

    fn set_read_only(&mut self, id: AclId, read_only: bool) -> Option<AclEvent> {
        let entry = self.entries.get_mut(&id)?;
        *entry = read_only;
        Some(AclEvent::Changed)
    }

    /// The access of a client with these IDs, or None if it is not
    /// allowed. An unknown ID (the peer credentials could not be read) is
    /// None. The user entry is used when there is one, even if the group
    /// entry would allow more.
    pub fn check(&self, uid: Option<u32>, gid: Option<u32>) -> Option<Access> {
        let read_only = match self.entries.get(&AclId::User(uid?)) {
            Some(&read_only) => read_only,
            None => *self.entries.get(&AclId::Group(gid?))?,
        };
        Some(if read_only {
            Access::ReadOnly
        } else {
            Access::ReadWrite
        })
    }

    /// The entries in order, users first, with whether each is read only.
    pub fn iter(&self) -> impl Iterator<Item = (AclId, bool)> + '_ {
        self.entries.iter().map(|(&id, &read_only)| (id, read_only))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: u32 = 1000;

    #[test]
    fn starts_with_root_and_the_owner() {
        let acl = ServerAcl::new(OWNER);
        let ids: Vec<AclId> = acl.iter().map(|(id, _)| id).collect();
        assert_eq!(ids, [AclId::User(0), AclId::User(OWNER)]);
        assert_eq!(acl.check(Some(OWNER), Some(1)), Some(Access::ReadWrite));
        assert_eq!(acl.check(Some(0), None), Some(Access::ReadWrite));
    }

    #[test]
    fn root_owner_has_one_entry() {
        let acl = ServerAcl::new(0);
        assert_eq!(acl.iter().count(), 1);
    }

    #[test]
    fn users_sort_before_groups() {
        let mut acl = ServerAcl::new(OWNER);
        acl.allow(AclId::Group(5));
        acl.allow(AclId::User(65534));
        acl.allow(AclId::Group(1));
        let ids: Vec<AclId> = acl.iter().map(|(id, _)| id).collect();
        assert_eq!(
            ids,
            [
                AclId::User(0),
                AclId::User(OWNER),
                AclId::User(65534),
                AclId::Group(1),
                AclId::Group(5),
            ]
        );
    }

    #[test]
    fn allow_twice_keeps_the_entry() {
        let mut acl = ServerAcl::new(OWNER);
        assert!(acl.allow(AclId::User(2)));
        acl.deny_write(AclId::User(2));
        assert!(!acl.allow(AclId::User(2)));
        assert_eq!(acl.check(Some(2), None), Some(Access::ReadOnly));
    }

    #[test]
    fn user_and_group_with_the_same_id_are_different() {
        let mut acl = ServerAcl::new(OWNER);
        acl.allow(AclId::Group(2));
        assert!(!acl.contains(AclId::User(2)));
        assert_eq!(acl.check(Some(2), Some(3)), None);
        assert_eq!(acl.check(Some(3), Some(2)), Some(Access::ReadWrite));
    }

    #[test]
    fn user_entry_wins_over_group() {
        let mut acl = ServerAcl::new(OWNER);
        acl.allow(AclId::User(2));
        acl.allow(AclId::Group(7));
        acl.deny_write(AclId::Group(7));
        assert_eq!(acl.check(Some(2), Some(7)), Some(Access::ReadWrite));
        assert_eq!(acl.check(Some(3), Some(7)), Some(Access::ReadOnly));
        acl.deny_write(AclId::User(2));
        acl.allow_write(AclId::Group(7));
        assert_eq!(acl.check(Some(2), Some(7)), Some(Access::ReadOnly));
    }

    #[test]
    fn unknown_ids_are_not_allowed() {
        let mut acl = ServerAcl::new(OWNER);
        acl.allow(AclId::Group(7));
        assert_eq!(acl.check(None, Some(7)), None);
        assert_eq!(acl.check(Some(2), None), None);
        // A user entry does not need the group.
        assert_eq!(acl.check(Some(OWNER), None), Some(Access::ReadWrite));
    }

    #[test]
    fn changes_return_events_only_for_existing_entries() {
        let mut acl = ServerAcl::new(OWNER);
        assert_eq!(acl.deny(AclId::User(2)), None);
        assert_eq!(acl.allow_write(AclId::User(2)), None);
        assert_eq!(acl.deny_write(AclId::Group(2)), None);
        acl.allow(AclId::User(2));
        assert_eq!(acl.deny_write(AclId::User(2)), Some(AclEvent::Changed));
        assert_eq!(acl.allow_write(AclId::User(2)), Some(AclEvent::Changed));
        assert_eq!(acl.deny(AclId::User(2)), Some(AclEvent::Changed));
        assert!(!acl.contains(AclId::User(2)));
        assert_eq!(acl.check(Some(2), None), None);
    }
}
