//! Fuzzy matching of what the user typed against candidate names.

/// How well `query` matches `label`; lower is better, `None` means no match.
///
/// * 0   exact-case prefix (`pri` → `print`)
/// * 100 case-insensitive prefix (`Pri` → `print`)
/// * 300 each query letter starts a word or hump (`gun` → `getUserName`)
/// * 500 plain subsequence (`gnm` → `getUserName`), only for queries of 2+ letters
pub(crate) fn score(query: &str, label: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    if label.starts_with(query) {
        // Ties between prefix matches are broken by length, so an exact
        // match (`for`) lands ahead of longer ones (`forr`).
        return Some(0);
    }
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let l: Vec<char> = label.chars().collect();
    let lower: Vec<char> = label.chars().flat_map(char::to_lowercase).collect();

    if lower.len() >= q.len() && lower[..q.len()] == q[..] {
        return Some(100 + (l.len() - q.len()).min(50) as u32);
    }
    if q.len() < 2 {
        return None;
    }

    // Humps: positions where a new word starts (after `_`, or a lower→upper change).
    let is_hump = |i: usize| -> bool {
        i == 0
            || l[i - 1] == '_'
            || l[i - 1] == '-'
            || (l[i].is_uppercase() && l[i - 1].is_lowercase())
            || (l[i - 1].is_ascii_digit() && !l[i].is_ascii_digit())
    };
    // Greedy: first query char must start the label or a hump, rest at humps in order.
    if lower.first() == q.first() {
        let mut qi = 1;
        let mut gaps = 0;
        for i in 1..l.len() {
            if qi < q.len() && lower[i] == q[qi] && is_hump(i) {
                qi += 1;
            } else if qi < q.len() {
                gaps += 1;
            }
        }
        if qi == q.len() {
            return Some(300 + gaps.min(100));
        }
    }

    // Plain subsequence.
    let mut qi = 0;
    let mut gaps = 0u32;
    let mut started = false;
    for &c in &lower {
        if qi < q.len() && c == q[qi] {
            qi += 1;
            started = true;
        } else if started && qi < q.len() {
            gaps += 1;
        }
    }
    (qi == q.len()).then(|| 500 + gaps.min(200))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_beats_humps_beats_subsequence() {
        let prefix = score("get", "getUserName").unwrap();
        let ci_prefix = score("Get", "getUserName").unwrap();
        let hump = score("gun", "getUserName").unwrap();
        let subseq = score("gnm", "getUserName").unwrap();
        assert!(prefix < ci_prefix && ci_prefix < hump && hump < subseq);
    }

    #[test]
    fn snake_case_humps() {
        assert!(score("hw", "hello_world").unwrap() < 500);
        assert!(score("hw", "hello_world").unwrap() >= 300);
    }

    #[test]
    fn rejects_non_matches_and_single_letter_fuzz() {
        assert!(score("xyz", "getUserName").is_none());
        // One letter must be a prefix, otherwise everything would match.
        assert!(score("u", "getUserName").is_none());
        assert!(score("g", "getUserName").is_some());
    }

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(score("", "anything"), Some(0));
    }
}
