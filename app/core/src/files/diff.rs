//! Changed-line marks for the Files panel: which lines of the current file are new since the base, and where lines
//! were removed (and how many). Only line numbers and counts leave this module, never base content: until the
//! engine-side key check ships, the panel does not show what a file held at the base (a key committed through a hard
//! link would live on there). Lines are compared with their line ends. The differing middle (after the common start
//! and end are cut) is aligned with a longest-common-subsequence table when small enough (`LCS_CELLS`); a larger one
//! is marked as removed whole, then added whole: still correct, only less tidy.

/// Largest LCS table (old lines times new lines in the differing middle).
const LCS_CELLS: usize = 4_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Same,
    Del,
    Add,
}

/// The edit script from `old` to `new`, as ops in order.
fn script(old: &[&str], new: &[&str]) -> Vec<Op> {
    let pre = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let rest = (old.len() - pre).min(new.len() - pre);
    let suf = old[pre..].iter().rev().zip(new[pre..].iter().rev()).take(rest).take_while(|(a, b)| a == b).count();
    let (a, b) = (&old[pre..old.len() - suf], &new[pre..new.len() - suf]);
    let mut out = vec![Op::Same; pre];
    if a.len().saturating_mul(b.len()) <= LCS_CELLS && !a.is_empty() && !b.is_empty() {
        // t[i][j] = LCS of a[i..] and b[j..].
        let w = b.len() + 1;
        let mut t = vec![0u32; (a.len() + 1) * w];
        for i in (0..a.len()).rev() {
            for j in (0..b.len()).rev() {
                t[i * w + j] = if a[i] == b[j] { t[(i + 1) * w + j + 1] + 1 } else { t[(i + 1) * w + j].max(t[i * w + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < a.len() && j < b.len() {
            if a[i] == b[j] {
                out.push(Op::Same);
                i += 1;
                j += 1;
            } else if t[(i + 1) * w + j] >= t[i * w + j + 1] {
                out.push(Op::Del);
                i += 1;
            } else {
                out.push(Op::Add);
                j += 1;
            }
        }
        out.extend(std::iter::repeat_n(Op::Del, a.len() - i));
        out.extend(std::iter::repeat_n(Op::Add, b.len() - j));
    } else {
        out.extend(std::iter::repeat_n(Op::Del, a.len()));
        out.extend(std::iter::repeat_n(Op::Add, b.len()));
    }
    out.extend(std::iter::repeat_n(Op::Same, suf));
    out
}

/// What changed in the current file, by line number (1-based) only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks {
    /// Current lines that are new or changed since the base.
    pub added: Vec<u32>,
    /// `(after, count)`: `count` base lines were removed just after current line `after` (0 = before the first line).
    pub removed: Vec<(u32, u32)>,
}

/// The marks from `old` (the base) to `new` (the current file).
pub fn marks(old: &str, new: &str) -> Marks {
    let ol: Vec<&str> = old.split_inclusive('\n').collect();
    let nl: Vec<&str> = new.split_inclusive('\n').collect();
    let mut m = Marks::default();
    let mut line = 0u32;
    for op in script(&ol, &nl) {
        match op {
            Op::Same => line += 1,
            Op::Add => {
                line += 1;
                m.added.push(line);
            }
            Op::Del => match m.removed.last_mut() {
                Some((after, n)) if *after == line => *n += 1,
                _ => m.removed.push((line, 1)),
            },
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_line_is_one_add_and_one_removal_before_it() {
        let m = marks("1\n2\n3\n", "1\ntwo\n3\n");
        assert_eq!(m, Marks { added: vec![2], removed: vec![(1, 1)] });
    }

    #[test]
    fn equal_texts_have_no_marks_and_pure_inserts_or_deletes_are_one_sided() {
        assert_eq!(marks("a\nb\n", "a\nb\n"), Marks::default());
        assert_eq!(marks("a\nc\n", "a\nb\nc\n"), Marks { added: vec![2], removed: vec![] });
        assert_eq!(marks("a\nb\nc\nd\n", "a\nd\n"), Marks { added: vec![], removed: vec![(1, 2)] });
        assert_eq!(marks("x\ny\n", "y\n"), Marks { added: vec![], removed: vec![(0, 1)] });
    }

    #[test]
    fn a_missing_final_newline_counts_as_a_change() {
        assert_eq!(marks("a\n", "a"), Marks { added: vec![1], removed: vec![(0, 1)] });
    }

    #[test]
    fn a_large_middle_falls_back_to_remove_then_add() {
        let old: String = (0..3000).map(|i| format!("o{i}\n")).collect();
        let new: String = (0..3000).map(|i| format!("n{i}\n")).collect();
        let m = marks(&old, &new);
        assert_eq!((m.added.len(), m.removed.as_slice()), (3000, &[(0, 3000)][..]));
    }
}
