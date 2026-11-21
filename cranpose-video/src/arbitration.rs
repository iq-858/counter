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

/// Releases a claim. If this was the newest claim in the group, the entry is
/// removed so a surviving player in the same group can be promoted on its next
/// check instead of staying muted forever. Also bounds the map: a group with no
/// players no longer occupies an entry.
pub(crate) fn release(group: u64, token: u64) {
    if token == 0 {
        return;
    }
    let mut owners = owners().lock().unwrap_or_else(|e| e.into_inner());
    if owners.get(&group).copied() == Some(token) {
        owners.remove(&group);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The owner registry is process-global, so every test claims its own group
    // to stay independent of the others (which the harness may run in parallel).

    #[test]
    fn a_newer_claim_in_the_same_group_stales_the_first_token() {
        let group = 1_001;
        let first = claim(group);
        let second = claim(group);

        assert_ne!(first, second, "each claim returns a fresh generation token");
        assert!(
            !is_current(group, first),
            "the first token is stale once a second player claims the group"
        );
        assert!(is_current(group, second), "the newest token is current");
    }

    #[test]
    fn claims_in_different_groups_do_not_affect_each_other() {
        let group_a = 1_002;
        let group_b = 1_003;
        let token_a = claim(group_a);
        let token_b_first = claim(group_b);
        let token_b_second = claim(group_b);

        assert!(
            is_current(group_a, token_a),
            "claims in another group must not stale this token"
        );
        assert!(
            !is_current(group_b, token_b_first),
            "claims inside group B stale each other"
        );
        assert!(
            is_current(group_b, token_b_second),
            "group B tracks its own newest claim"
        );
    }

    #[test]
    fn an_unclaimed_zero_token_is_treated_as_current() {
        assert_eq!(DEFAULT_GROUP, 0, "the implicit default group is group 0");

        let group = 1_004;
        assert!(
            is_current(group, 0),
            "token 0 means 'never claimed' and must behave as current"
        );

        claim(group);
        assert!(
            is_current(group, 0),
            "token 0 stays current even after the group has been claimed"
        );
    }

    #[test]
    fn many_claims_leave_only_the_last_token_current() {
        let group = 1_005;
        let tokens: Vec<u64> = (0..32).map(|_| claim(group)).collect();
        let last = *tokens.last().expect("at least one claim was made");

        assert!(
            tokens[..tokens.len() - 1]
                .iter()
                .all(|&token| !is_current(group, token)),
            "every earlier generation is stale"
        );
        assert!(
            is_current(group, last),
            "only the most recent claim is current"
        );
    }
}
