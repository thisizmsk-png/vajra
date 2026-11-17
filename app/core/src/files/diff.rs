//! A unified line diff of two texts the core already holds, so git never reopens a working file for the Files panel.
//! Lines are compared with their line ends, so a missing final newline counts as a change. The middle that differs
//! (after the common start and end are cut) is aligned with a longest-common-subsequence table when it is small enough
//! (`LCS_CELLS`); a larger one is shown as removed whole, then added whole: still correct, only less tidy.

/// Largest LCS table (old lines times new lines in the differing middle).
const LCS_CELLS: usize = 4_000_000;
/// Context lines around each change.
const CONTEXT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Same,
    Del,
    Add,
}

/// The edit script from `old` to `new`, as (op, line) in order.
fn script<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(Op, &'a str)> {
    let pre = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let rest = (old.len() - pre).min(new.len() - pre);
    let suf = old[pre..].iter().rev().zip(new[pre..].iter().rev()).take(rest).take_while(|(a, b)| a == b).count();
    let (a, b) = (&old[pre..old.len() - suf], &new[pre..new.len() - suf]);
    let mut out: Vec<(Op, &str)> = old[..pre].iter().map(|l| (Op::Same, *l)).collect();
    if a.len().saturating_mul(b.len()) <= LCS_CELLS && !a.is_empty() && !b.is_empty() {
        // t[i][j] = LCS of a[i..] and b[j..]; both sides are at most LCS_CELLS long, the LCS fits in u32.
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
                out.push((Op::Same, a[i]));
                i += 1;
                j += 1;
            } else if t[(i + 1) * w + j] >= t[i * w + j + 1] {
                out.push((Op::Del, a[i]));
                i += 1;
            } else {
                out.push((Op::Add, b[j]));
                j += 1;
            }
        }
        out.extend(a[i..].iter().map(|l| (Op::Del, *l)));
        out.extend(b[j..].iter().map(|l| (Op::Add, *l)));
    } else {
        out.extend(a.iter().map(|l| (Op::Del, *l)));
        out.extend(b.iter().map(|l| (Op::Add, *l)));
    }
    out.extend(old[old.len() - suf..].iter().map(|l| (Op::Same, *l)));
    out
}

fn push_line(out: &mut String, mark: char, line: &str) {
    out.push(mark);
    match line.strip_suffix('\n') {
        Some(l) => {
            out.push_str(l);
            out.push('\n');
        }
        None => {
            out.push_str(line);
            out.push_str("\n\\ No newline at end of file\n");
        }
    }
}

/// `old` is `None` for a file new since the base, `new` is `None` for one deleted since. Empty when they are equal.
pub fn unified(path: &str, old: Option<&str>, new: Option<&str>) -> String {
    let (o, n) = (old.unwrap_or(""), new.unwrap_or(""));
    if old.is_some() && old == new {
        return String::new();
    }
    let ol: Vec<&str> = o.split_inclusive('\n').collect();
    let nl: Vec<&str> = n.split_inclusive('\n').collect();
    let ops = script(&ol, &nl);
    let mut out = format!("diff --git a/{path} b/{path}\n");
    if old.is_none() {
        out.push_str("new file\n");
    } else if new.is_none() {
        out.push_str("deleted file\n");
    }
    out.push_str(&format!("--- {}\n+++ {}\n", if old.is_some() { format!("a/{path}") } else { "/dev/null".into() }, if new.is_some() { format!("b/{path}") } else { "/dev/null".into() }));
    // Positions (old line, new line) before each op.
    let mut pos = Vec::with_capacity(ops.len() + 1);
    let (mut oi, mut ni) = (0usize, 0usize);
    for (op, _) in &ops {
        pos.push((oi, ni));
        match op {
            Op::Same => {
                oi += 1;
                ni += 1;
            }
            Op::Del => oi += 1,
            Op::Add => ni += 1,
        }
    }
    pos.push((oi, ni));
    let changed: Vec<usize> = ops.iter().enumerate().filter(|(_, (op, _))| *op != Op::Same).map(|(i, _)| i).collect();
    let mut k = 0;
    while k < changed.len() {
        let start = changed[k].saturating_sub(CONTEXT);
        let mut last = changed[k];
        while k + 1 < changed.len() && changed[k + 1] - last <= 2 * CONTEXT {
            k += 1;
            last = changed[k];
        }
        let end = (last + CONTEXT + 1).min(ops.len());
        let slice = &ops[start..end];
        let oc = slice.iter().filter(|(op, _)| *op != Op::Add).count();
        let nc = slice.iter().filter(|(op, _)| *op != Op::Del).count();
        let (os, ns) = pos[start];
        out.push_str(&format!("@@ -{},{oc} +{},{nc} @@\n", if oc == 0 { os } else { os + 1 }, if nc == 0 { ns } else { ns + 1 }));
        for (op, line) in slice {
            push_line(&mut out, match op { Op::Same => ' ', Op::Del => '-', Op::Add => '+' }, line);
        }
        k += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_line_gets_one_hunk_with_context() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n";
        let new = "1\n2\n3\n4\nfive\n6\n7\n8\n";
        assert_eq!(unified("a.txt", Some(old), Some(new)), "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -2,7 +2,7 @@\n 2\n 3\n 4\n-5\n+five\n 6\n 7\n 8\n");
    }

    #[test]
    fn equal_texts_have_no_diff_and_far_changes_get_two_hunks() {
        assert_eq!(unified("a", Some("x\n"), Some("x\n")), "");
        let old: String = (1..=30).map(|i| format!("{i}\n")).collect();
        let new = old.replacen("\n2\n", "\ntwo\n", 1).replacen("\n29\n", "\ntwenty-nine\n", 1);
        assert_eq!(unified("a", Some(&old), Some(&new)).matches("\n@@ ").count(), 2);
    }

    #[test]
    fn new_deleted_and_no_final_newline() {
        assert_eq!(unified("n", None, Some("a\nb")), "diff --git a/n b/n\nnew file\n--- /dev/null\n+++ b/n\n@@ -0,0 +1,2 @@\n+a\n+b\n\\ No newline at end of file\n");
        assert_eq!(unified("d", Some("a\n"), None), "diff --git a/d b/d\ndeleted file\n--- a/d\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-a\n");
    }

    #[test]
    fn a_large_middle_falls_back_to_remove_then_add() {
        let old: String = (0..3000).map(|i| format!("o{i}\n")).collect();
        let new: String = (0..3000).map(|i| format!("n{i}\n")).collect();
        let d = unified("big", Some(&old), Some(&new));
        assert_eq!((d.matches("\n-o").count(), d.matches("\n+n").count()), (3000, 3000));
    }
}
