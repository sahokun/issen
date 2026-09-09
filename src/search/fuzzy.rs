/// Checks whether the query appears as a subsequence (characters in order, not
/// necessarily contiguous) of the target, and returns a score if it does (higher is a
/// better match). Bonuses apply for consecutive matches and matches at word
/// boundaries (right after a separator, or a camelCase transition).
pub fn fuzzy_match(query: &str, target: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let target_chars: Vec<char> = target.chars().collect();

    let mut score = 0i32;
    let mut target_idx = 0usize;
    let mut prev_matched_idx: Option<usize> = None;

    for qc in query.chars() {
        let qc_lower = qc.to_ascii_lowercase();
        let mut found = None;

        while target_idx < target_chars.len() {
            if target_chars[target_idx].to_ascii_lowercase() == qc_lower {
                found = Some(target_idx);
                break;
            }
            target_idx += 1;
        }

        let idx = found?;

        let is_boundary = idx == 0
            || !target_chars[idx - 1].is_alphanumeric()
            || (target_chars[idx].is_uppercase() && target_chars[idx - 1].is_lowercase());
        let is_consecutive = prev_matched_idx == Some(idx.wrapping_sub(1));

        score += 1;
        if is_boundary {
            score += 8;
        }
        if is_consecutive {
            score += 5;
        }

        prev_matched_idx = Some(idx);
        target_idx += 1;
    }

    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_subsequence_case_insensitively() {
        assert!(fuzzy_match("chr", "Google Chrome").is_some());
        assert!(fuzzy_match("gcr", "Google Chrome").is_some());
        assert!(fuzzy_match("xyz", "Google Chrome").is_none());
    }

    #[test]
    fn word_boundary_scores_higher_than_mid_word() {
        let boundary = fuzzy_match("c", "Google Chrome").unwrap();
        let mid_word = fuzzy_match("r", "Google Chrome").unwrap();
        assert!(boundary > mid_word);
    }

    #[test]
    fn empty_query_matches_everything_with_zero_score() {
        assert_eq!(fuzzy_match("", "anything"), Some(0));
    }
}
