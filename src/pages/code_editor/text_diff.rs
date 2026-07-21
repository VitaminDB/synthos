#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Context,
    Removed,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
}

const LCS_CELL_BUDGET: usize = 4_000_000;

pub fn unified_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let n = a.len();
    let m = b.len();

    if n.saturating_mul(m) > LCS_CELL_BUDGET {
        return linear_diff(&a, &b);
    }

    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut out = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(DiffLine {
                kind: DiffKind::Context,
                text: a[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(DiffLine {
                kind: DiffKind::Removed,
                text: a[i].to_string(),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                kind: DiffKind::Added,
                text: b[j].to_string(),
            });
            j += 1;
        }
    }
    while i < n {
        out.push(DiffLine {
            kind: DiffKind::Removed,
            text: a[i].to_string(),
        });
        i += 1;
    }
    while j < m {
        out.push(DiffLine {
            kind: DiffKind::Added,
            text: b[j].to_string(),
        });
        j += 1;
    }
    out
}

fn linear_diff(a: &[&str], b: &[&str]) -> Vec<DiffLine> {
    let n = a.len();
    let m = b.len();
    let mut head = 0;
    while head < n && head < m && a[head] == b[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < n - head && tail < m - head && a[n - 1 - tail] == b[m - 1 - tail] {
        tail += 1;
    }

    let mut out = Vec::with_capacity(n + m);
    for line in &a[..head] {
        out.push(DiffLine {
            kind: DiffKind::Context,
            text: line.to_string(),
        });
    }
    for line in &a[head..n - tail] {
        out.push(DiffLine {
            kind: DiffKind::Removed,
            text: line.to_string(),
        });
    }
    for line in &b[head..m - tail] {
        out.push(DiffLine {
            kind: DiffKind::Added,
            text: line.to_string(),
        });
    }
    for line in &a[n - tail..] {
        out.push(DiffLine {
            kind: DiffKind::Context,
            text: line.to_string(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lines: &[DiffLine]) -> Vec<DiffKind> {
        lines.iter().map(|l| l.kind).collect()
    }

    #[test]
    fn identical_is_all_context() {
        let d = unified_lines("a\nb\nc", "a\nb\nc");
        assert_eq!(kinds(&d), vec![DiffKind::Context; 3]);
    }

    #[test]
    fn pure_addition() {
        let d = unified_lines("a\nc", "a\nb\nc");
        assert_eq!(
            d,
            vec![
                DiffLine { kind: DiffKind::Context, text: "a".into() },
                DiffLine { kind: DiffKind::Added, text: "b".into() },
                DiffLine { kind: DiffKind::Context, text: "c".into() },
            ]
        );
    }

    #[test]
    fn pure_removal() {
        let d = unified_lines("a\nb\nc", "a\nc");
        assert_eq!(
            d,
            vec![
                DiffLine { kind: DiffKind::Context, text: "a".into() },
                DiffLine { kind: DiffKind::Removed, text: "b".into() },
                DiffLine { kind: DiffKind::Context, text: "c".into() },
            ]
        );
    }

    #[test]
    fn replacement_shows_both() {
        let d = unified_lines("a\nX\nc", "a\nY\nc");
        assert_eq!(kinds(&d), vec![DiffKind::Context, DiffKind::Removed, DiffKind::Added, DiffKind::Context]);
    }

    #[test]
    fn empty_old_all_added() {
        let d = unified_lines("", "a\nb");
        assert_eq!(kinds(&d), vec![DiffKind::Added, DiffKind::Added]);
    }

    #[test]
    fn linear_keeps_common_prefix_and_suffix() {
        let a = vec!["h", "x", "y", "t"];
        let b = vec!["h", "z", "t"];
        let d = linear_diff(&a, &b);
        assert_eq!(
            d,
            vec![
                DiffLine { kind: DiffKind::Context, text: "h".into() },
                DiffLine { kind: DiffKind::Removed, text: "x".into() },
                DiffLine { kind: DiffKind::Removed, text: "y".into() },
                DiffLine { kind: DiffKind::Added, text: "z".into() },
                DiffLine { kind: DiffKind::Context, text: "t".into() },
            ]
        );
    }

    #[test]
    fn linear_prefix_of_other() {
        let a = vec!["a", "b"];
        let b = vec!["a", "b", "c", "d"];
        let d = linear_diff(&a, &b);
        assert_eq!(kinds(&d), vec![DiffKind::Context, DiffKind::Context, DiffKind::Added, DiffKind::Added]);
    }
}
