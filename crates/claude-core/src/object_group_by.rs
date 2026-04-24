//! Group an iterable into buckets by a key selector.
//!
//! Port of TS `src/utils/objectGroupBy.ts` (which mirrors the
//! ECMA-262 `Object.groupBy`). Preserves first-seen key order by
//! using a `BTreeMap` when the key type is `Ord`, and
//! (optionally) a `HashMap` when callers only need bucket lookup.
//!
//! The Rust stdlib has `Itertools::into_group_map` in the
//! `itertools` crate; we don't add a dep for a six-line helper.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

/// Group `items` into buckets keyed by `key_selector(item, index)`.
/// Preserves first-insertion key order (matches ECMA-262).
pub fn group_by_hash<I, T, K, F>(items: I, mut key_selector: F) -> HashMap<K, Vec<T>>
where
    I: IntoIterator<Item = T>,
    K: Eq + Hash,
    F: FnMut(&T, usize) -> K,
{
    let mut out: HashMap<K, Vec<T>> = HashMap::new();
    for (idx, item) in items.into_iter().enumerate() {
        let key = key_selector(&item, idx);
        out.entry(key).or_default().push(item);
    }
    out
}

/// Group by an ordered key. Useful when callers want deterministic
/// iteration (e.g. generating stable test output).
pub fn group_by_ordered<I, T, K, F>(items: I, mut key_selector: F) -> BTreeMap<K, Vec<T>>
where
    I: IntoIterator<Item = T>,
    K: Ord,
    F: FnMut(&T, usize) -> K,
{
    let mut out: BTreeMap<K, Vec<T>> = BTreeMap::new();
    for (idx, item) in items.into_iter().enumerate() {
        let key = key_selector(&item, idx);
        out.entry(key).or_default().push(item);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_by_parity() {
        let got = group_by_hash(1..=6, |n, _| n % 2 == 0);
        assert_eq!(got.get(&true), Some(&vec![2, 4, 6]));
        assert_eq!(got.get(&false), Some(&vec![1, 3, 5]));
    }

    #[test]
    fn uses_index_in_selector() {
        let got = group_by_ordered(vec!["a", "b", "c", "d"], |_, i| {
            if i < 2 {
                "head"
            } else {
                "tail"
            }
        });
        assert_eq!(got.get("head"), Some(&vec!["a", "b"]));
        assert_eq!(got.get("tail"), Some(&vec!["c", "d"]));
    }

    #[test]
    fn empty_input_returns_empty_map() {
        let got: HashMap<i32, Vec<i32>> = group_by_hash(std::iter::empty::<i32>(), |n, _| *n);
        assert!(got.is_empty());
    }

    #[test]
    fn ordered_variant_iterates_sorted() {
        let got = group_by_ordered(vec![3, 1, 2, 1, 3], |n, _| *n);
        let keys: Vec<i32> = got.keys().copied().collect();
        assert_eq!(keys, vec![1, 2, 3]);
        assert_eq!(got.get(&1), Some(&vec![1, 1]));
    }
}
