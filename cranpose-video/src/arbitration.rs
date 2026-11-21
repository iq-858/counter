//! Audio-output arbitration shared by every audio owner in the library.
//!
//! A "group" is a set of players whose audio is mutually exclusive: when a
//! newer player claims a group, older players in it mute themselves. Players in
//! different groups keep their own sound and can play at the same time.
//!
//! Group 0 is the implicit default. A feed that only ever plays one clip at a
//! time needs no cooperation at all — every player is born in group 0 and the
//! newest claim wins. An application that deliberately plays two clips with
//! sound gives them distinct groups through `VideoPlayer::set_audio_group`.
//!
//! The registry holds only small generation tokens, never player objects, so
//! it is safe to query from an audio or decode thread and needs no callbacks.

use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};

/// The default group: newest claim wins, older players mute.
pub(crate) const DEFAULT_GROUP: u64 = 0;

static OWNERS: OnceLock<Mutex<BTreeMap<u64, u64>>> = OnceLock::new();

fn owners() -> &'static Mutex<BTreeMap<u64, u64>> {
    OWNERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Claims ownership of `group`, returning a new monotonic token.
///
/// `0` is reserved as "never claimed" so a player that has not made sound yet
/// does not suppress anything.
pub(crate) fn claim(group: u64) -> u64 {
    let mut owners = owners().lock().unwrap_or_else(|e| e.into_inner());
    let next = owners.get(&group).copied().unwrap_or(0).wrapping_add(1);
    // 0 is the sentinel for "not claimed"; skip it after a wrap.
    let next = if next == 0 { 1 } else { next };
    owners.insert(group, next);
    next
}

/// Whether `token` is still the newest claim in its group.
pub(crate) fn is_current(group: u64, token: u64) -> bool {
    if token == 0 {
        // Never claimed: behave as current so a player is not muted before its
        // first audible use.
        return true;
    }
    let owners = owners().lock().unwrap_or_else(|e| e.into_inner());
    owners.get(&group).copied() == Some(token)
}
