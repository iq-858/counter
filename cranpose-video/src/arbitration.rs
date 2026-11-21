//! Audio-output arbitration shared by every audio owner in the library.
//!
//! A "group" is a set of players whose audio is mutually exclusive: the most
//! recently claimed player in a group is the only audible one. Players in
//! different groups keep their own sound and can play at the same time.
//!
//! Group 0 is the implicit default. A feed that only ever plays one clip at a
//! time needs no cooperation at all — every player is born in group 0 and the
//! newest claim wins. An application that deliberately plays two clips with
//! sound gives them distinct groups via `VideoPlayer::set_audio_group`.
//!
//! Each group keeps a stack of live claim tokens. The last token is the current
//! owner; releasing it promotes the previous one automatically, so a surviving
//! player is never left muted forever after the owner closes. The registry holds
//! only small integers, never player objects, so it is safe from any thread.

use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};

/// The default group: newest claim wins, older players mute.
pub(crate) const DEFAULT_GROUP: u64 = 0;

/// A group's live claim tokens, newest last, plus a process-wide unique source
/// for tokens so a token is never reused across groups.
struct Registry {
    groups: BTreeMap<u64, Vec<u64>>,
    next_token: u64,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            groups: BTreeMap::new(),
            next_token: 1,
        })
    })
}

/// Claims ownership of `group`, returning a globally unique token.
pub(crate) fn claim(group: u64) -> u64 {
    let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    let token = registry.next_token;
    registry.next_token = registry.next_token.wrapping_add(1).max(1);
    registry.groups.entry(group).or_default().push(token);
    token
}

/// Whether `token` is still the newest live claim in its group.
pub(crate) fn is_current(group: u64, token: u64) -> bool {
    if token == 0 {
        // Never claimed: behave as current so a player is not muted before its
        // first audible use.
        return true;
    }
    let registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    registry
        .groups
        .get(&group)
        .and_then(|stack| stack.last())
        .copied()
        == Some(token)
}

/// Releases a claim. If it was the newest, the previous live claim in the group
/// becomes the owner automatically (promotion). When the group is empty its
/// entry is removed, so the map stays bounded by live players.
pub(crate) fn release(group: u64, token: u64) {
    if token == 0 {
        return;
    }
    let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(stack) = registry.groups.get_mut(&group) {
        if let Some(at) = stack.iter().rposition(|value| *value == token) {
            stack.remove(at);
        }
        if stack.is_empty() {
            registry.groups.remove(&group);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The owner registry is process-global, so every test claims its own group
    // to stay independent of the others (which the harness may run in parallel).

    #[test]
    fn a_newer_claim_in_the_same_group_stales_the_first_token() {
        let group = 2001;
        let first = claim(group);
        let second = claim(group);
        assert!(!is_current(group, first));
        assert!(is_current(group, second));
        release(group, second);
        release(group, first);
    }

    #[test]
    fn releasing_the_owner_promotes_the_previous_claim() {
        let group = 2002;
        let first = claim(group);
        let second = claim(group);
        assert!(is_current(group, second));
        release(group, second);
        // The survivor is promoted instead of staying muted forever.
        assert!(is_current(group, first));
        release(group, first);
    }

    #[test]
    fn claims_in_different_groups_do_not_affect_each_other() {
        let group_a = 2003;
        let group_b = 2004;
        let a = claim(group_a);
        let b = claim(group_b);
        let a2 = claim(group_a);
        assert!(!is_current(group_a, a));
        assert!(is_current(group_a, a2));
        assert!(
            is_current(group_b, b),
            "a claim in another group must not mute b"
        );
        release(group_a, a2);
        release(group_a, a);
        release(group_b, b);
    }

    #[test]
    fn an_unclaimed_zero_token_is_treated_as_current() {
        assert_eq!(DEFAULT_GROUP, 0);
        assert!(is_current(2005, 0));
    }

    #[test]
    fn many_claims_leave_only_the_last_token_current() {
        let group = 2006;
        let tokens: Vec<u64> = (0..32).map(|_| claim(group)).collect();
        for token in &tokens[..tokens.len() - 1] {
            assert!(!is_current(group, *token));
        }
        assert!(is_current(group, *tokens.last().unwrap()));
        for token in tokens.iter().rev() {
            release(group, *token);
        }
    }
}
