//! "You may like": the similar titles of what the user watched lately, merged.

use std::collections::{HashMap, HashSet};

/// Merges the similar lists of several titles: a title gains 1 for first place in a list, 1/2
/// for second and so on, summed over the lists; ties keep the order they first appeared in.
pub fn merge_similar(lists: &[Vec<i64>], owned: &HashSet<i64>, limit: usize) -> Vec<i64> {
    let mut score: HashMap<i64, f64> = HashMap::new();
    let mut order: Vec<i64> = Vec::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            if owned.contains(id) {
                continue;
            }
            if !score.contains_key(id) {
                order.push(*id);
            }
            *score.entry(*id).or_default() += 1.0 / (rank as f64 + 1.0);
        }
    }
    // A stable sort keeps the order of first appearance among equal scores.
    order.sort_by(|a, b| score[b].total_cmp(&score[a]));
    order.truncate(limit);
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_similar_to_several_seeds_and_near_the_top_come_first() {
        let lists = vec![vec![10, 11, 12], vec![11, 13], vec![14, 11]];
        assert_eq!(
            merge_similar(&lists, &HashSet::new(), 10),
            vec![11, 10, 14, 13, 12]
        );
    }

    #[test]
    fn what_the_user_already_has_is_left_out() {
        let lists = vec![vec![10, 11, 12]];
        let owned = HashSet::from([10, 12]);
        assert_eq!(merge_similar(&lists, &owned, 10), vec![11]);
    }

    #[test]
    fn the_list_is_cut_to_the_limit() {
        let lists = vec![(1..=30).collect::<Vec<i64>>()];
        assert_eq!(
            merge_similar(&lists, &HashSet::new(), 5),
            vec![1, 2, 3, 4, 5]
        );
    }
}
