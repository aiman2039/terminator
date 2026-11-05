//! Lane assignment. Each row reads `@col:color up[...] down[...]
//! through[...]`, and each line reads `col:color`.
use super::*;

fn lines(lines: &[LaneLine]) -> String {
    lines
        .iter()
        .map(|line| format!("{}:{}", line.col, line.color))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shown(history: &[(&str, &[&str])]) -> Vec<String> {
    graph_rows(history.iter().map(|(hash, parents)| (*hash, *parents)))
        .iter()
        .map(|row| {
            format!(
                "@{}:{} up[{}] down[{}] through[{}]",
                row.col,
                row.color,
                lines(&row.up),
                lines(&row.down),
                lines(&row.through)
            )
        })
        .collect()
}

#[test]
fn linear_history_stays_in_one_lane() {
    // C - B - A
    assert_eq!(
        shown(&[("C", &["B"]), ("B", &["A"]), ("A", &[])]),
        [
            "@0:0 up[] down[0:0] through[]",
            "@0:0 up[0:0] down[0:0] through[]",
            "@0:0 up[0:0] down[] through[]",
        ]
    );
}

#[test]
fn a_branch_and_its_merge_open_a_lane_that_joins_at_the_fork() {
    // M merges B into C; both fork from A (`git log --date-order`).
    //   *   M
    //   |\
    //   | * B
    //   * | C
    //   |/
    //   *   A
    assert_eq!(
        shown(&[("M", &["C", "B"]), ("B", &["A"]), ("C", &["A"]), ("A", &[])]),
        [
            "@0:0 up[] down[0:0 1:1] through[]",
            "@1:1 up[1:1] down[1:1] through[0:0]",
            "@0:0 up[0:0] down[0:0] through[1:1]",
            "@0:0 up[0:0 1:1] down[] through[]",
        ]
    );
}

#[test]
fn unmerged_branch_tips_open_their_own_lane_and_color() {
    // `feature` (F) and `main` (M) both grow from A; nothing merges.
    assert_eq!(
        shown(&[("F", &["A"]), ("M", &["A"]), ("A", &[])]),
        [
            "@0:0 up[] down[0:0] through[]",
            "@1:1 up[] down[1:1] through[0:0]",
            "@0:0 up[0:0 1:1] down[] through[]",
        ]
    );
}

#[test]
fn an_octopus_merge_opens_a_lane_for_each_extra_parent() {
    // O merges A, B, and C, which all grow from R.
    assert_eq!(
        shown(&[
            ("O", &["A", "B", "C"]),
            ("A", &["R"]),
            ("B", &["R"]),
            ("C", &["R"]),
            ("R", &[]),
        ]),
        [
            "@0:0 up[] down[0:0 1:1 2:2] through[]",
            "@0:0 up[0:0] down[0:0] through[1:1 2:2]",
            "@1:1 up[1:1] down[1:1] through[0:0 2:2]",
            "@2:2 up[2:2] down[2:2] through[0:0 1:1]",
            "@0:0 up[0:0 1:1 2:2] down[] through[]",
        ]
    );
}

#[test]
fn criss_cross_merges_join_the_lanes_that_already_expect_their_parents() {
    // D merges C into B, E merges B into C; B and C grow from A.
    assert_eq!(
        shown(&[
            ("D", &["B", "C"]),
            ("E", &["C", "B"]),
            ("B", &["A"]),
            ("C", &["A"]),
            ("A", &[]),
        ]),
        [
            "@0:0 up[] down[0:0 1:1] through[]",
            // E's first parent keeps E's own lane; its merge parent B
            // joins lane 0, which D already opened toward B.
            "@2:2 up[] down[2:2 0:0] through[0:0 1:1]",
            "@0:0 up[0:0] down[0:0] through[1:1 2:2]",
            // Both lanes that expect C end at its dot.
            "@1:1 up[1:1 2:2] down[1:1] through[0:0]",
            "@0:0 up[0:0 1:1] down[] through[]",
        ]
    );
}

#[test]
fn parents_outside_the_list_keep_their_lanes_open() {
    // The log stops before X and A: both lanes run off the bottom.
    let rows = shown(&[("Y", &["X"]), ("B", &["A"])]);
    assert_eq!(
        rows,
        [
            "@0:0 up[] down[0:0] through[]",
            "@1:1 up[] down[1:1] through[0:0]",
        ]
    );
}

#[test]
fn a_root_frees_its_column_for_the_next_branch_tip() {
    // Two unrelated histories: the second reuses column 0.
    assert_eq!(
        shown(&[("B", &["A"]), ("A", &[]), ("Q", &["P"]), ("P", &[])]),
        [
            "@0:0 up[] down[0:0] through[]",
            "@0:0 up[0:0] down[] through[]",
            "@0:1 up[] down[0:1] through[]",
            "@0:1 up[0:1] down[] through[]",
        ]
    );
}

#[test]
fn a_merge_may_reuse_a_column_that_ends_at_its_dot() {
    // X and Y both expect M; Y's lane ends at M, and M's merge parent B
    // takes that free column instead of widening the graph.
    assert_eq!(
        shown(&[
            ("X", &["M"]),
            ("Y", &["M"]),
            ("M", &["A", "B"]),
            ("B", &[]),
            ("A", &[]),
        ]),
        [
            "@0:0 up[] down[0:0] through[]",
            "@1:1 up[] down[1:1] through[0:0]",
            "@0:0 up[0:0 1:1] down[0:0 1:2] through[]",
            "@1:2 up[1:2] down[] through[0:0]",
            "@0:0 up[0:0] down[] through[]",
        ]
    );
}

#[test]
fn a_repeated_parent_draws_one_line() {
    assert_eq!(
        shown(&[("M", &["A", "A"]), ("A", &[])]),
        [
            "@0:0 up[] down[0:0] through[]",
            "@0:0 up[0:0] down[] through[]",
        ]
    );
}

#[test]
fn row_width_counts_the_widest_column() {
    let rows = graph_rows([("O", &["A", "B", "C"][..]), ("A", &[][..])]);
    let widths: Vec<usize> = rows.iter().map(GraphRow::width).collect();
    // The second row still passes lanes 1 and 2 through.
    assert_eq!(widths, [3, 3]);
    assert_eq!(GraphRow::default().width(), 1);
}

#[test]
fn a_thousand_commit_history_is_laid_out_in_one_pass() {
    // A long first-parent line with a side branch merged every 10 commits.
    let mut history: Vec<(String, Vec<String>)> = Vec::new();
    for index in 0..1000_usize {
        let parent = format!("c{}", index.saturating_add(1));
        let parents = if index.is_multiple_of(10) {
            vec![parent, format!("s{index}")]
        } else {
            vec![parent]
        };
        history.push((format!("c{index}"), parents));
    }
    let rows = graph_rows(
        history
            .iter()
            .map(|(hash, parents)| (hash.as_str(), parents.as_slice())),
    );
    assert_eq!(rows.len(), 1000);
    assert!(rows.iter().all(|row| row.col == 0));
    // Side commits never appear, so their lanes stay open to the bottom.
    assert_eq!(rows.last().map(GraphRow::width), Some(101));
}
