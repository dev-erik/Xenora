// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[test]
fn source_budget_rejects_before_growth() {
    let mut source = "x".repeat(MAX_MSL_BYTES - 1);
    append_msl(&mut source, "x").unwrap();
    assert!(append_msl(&mut source, "x").is_err());
    assert_eq!(source.len(), MAX_MSL_BYTES);
}
#[test]
fn node_budget_rejects_before_emission() {
    let ops = vec![(0, Op::Load(None))];
    let mut source = String::new();
    let mut nodes = ReplayBudget {
        nodes: MAX_REPLAY_NODES,
        work: 0,
        ..ReplayBudget::default()
    };
    let mut replay = Replay::new(
        &mut source,
        &ops,
        &[false],
        1,
        &mut nodes,
        "lane".into(),
        "lane_index".into(),
        false,
    );
    assert!(
        replay
            .value(Value(0), 0, 0)
            .unwrap_err()
            .to_string()
            .contains("8192")
    );
    assert!(source.is_empty());
    assert_eq!(nodes.nodes, MAX_REPLAY_NODES);
}
#[test]
fn ordinary_ssa_depth_is_not_context_depth_and_memo_is_reused() {
    let mut ops = vec![(0, Op::Load(None))];
    for i in 0..100 {
        ops.push((0, Op::Binary(Binary::Xor, Value(i), Value(i))));
    }
    let uniform = vec![false; ops.len()];
    let mut source = String::new();
    let mut nodes = ReplayBudget::default();
    let mut replay = Replay::new(
        &mut source,
        &ops,
        &uniform,
        ops.len(),
        &mut nodes,
        "lane".into(),
        "lane_index".into(),
        false,
    );
    replay.value(Value(100), 0, 0).unwrap();
    assert_eq!(replay.budget.nodes, 101);
    replay.value(Value(100), 0, 0).unwrap();
    assert_eq!(replay.budget.nodes, 101);
}
#[test]
fn repeated_context_key_reuses_selected_lane_temporary() {
    let selection = GatherSelection::Address {
        value: Value(0),
        offset: 0,
    };
    let ops = vec![
        (0, Op::Load(None)),
        (
            0,
            Op::Gather {
                source: Value(0),
                selection,
                exec: u32::MAX,
            },
        ),
        (
            0,
            Op::Gather {
                source: Value(0),
                selection,
                exec: u32::MAX,
            },
        ),
    ];
    let mut source = String::new();
    let mut nodes = ReplayBudget::default();
    let mut replay = Replay::new(
        &mut source,
        &ops,
        &[false; 3],
        3,
        &mut nodes,
        "lane".into(),
        "lane_index".into(),
        false,
    );
    replay.value(Value(1), 0, 0).unwrap();
    replay.value(Value(2), 0, 0).unwrap();
    assert_eq!(replay.contexts.len(), 2);
}
#[test]
fn context_depth_is_bounded_independently_of_source_size() {
    let mut ops = vec![(0, Op::Load(None))];
    for i in 0..65 {
        ops.push((
            0,
            Op::Gather {
                source: Value(i),
                selection: GatherSelection::Table([0; 32]),
                exec: u32::MAX,
            },
        ));
    }
    let uniform = vec![false; ops.len()];
    let mut source = String::new();
    let mut nodes = ReplayBudget::default();
    let mut replay = Replay::new(
        &mut source,
        &ops,
        &uniform,
        ops.len(),
        &mut nodes,
        "lane".into(),
        "lane_index".into(),
        false,
    );
    assert!(
        replay
            .value(Value(65), 0, 0)
            .unwrap_err()
            .to_string()
            .contains("depth exceeds 64")
    );
    assert!(source.len() < MAX_MSL_BYTES);
}

#[test]
fn weighted_budget_exact_boundary_and_overflow_leave_counter_unchanged() {
    let mut budget = ReplayBudget::default();
    budget.charge(256, 32).unwrap();
    assert_eq!(budget.work, 8192);
    assert!(budget.charge(1, 1).is_err());
    assert_eq!(budget.work, 8192);
    let mut budget = ReplayBudget::default();
    assert!(budget.charge(usize::MAX, 32).is_err());
    assert_eq!(budget.work, 0);
}
#[test]
fn comparison_scatter_multiplies_body_work_and_memoizes_shared_source() {
    let ops = vec![
        (0, Op::Load(None)),
        (
            0,
            Op::Scatter {
                address: Value(0),
                data: Value(0),
                offset: 0,
                exec: u32::MAX,
            },
        ),
    ];
    let mut source = String::new();
    let mut budget = ReplayBudget::default();
    let mut replay = Replay::new(
        &mut source,
        &ops,
        &[false; 2],
        2,
        &mut budget,
        "lane".into(),
        "lane_index".into(),
        false,
    );
    replay.multiplier = 32;
    replay.value(Value(1), 0, 0).unwrap();
    // Scatter node, 32 writer iterations with six control operations, one shared load.
    assert_eq!(replay.budget.work, 32 + 32 * 32 * 7);
    let old = replay.budget.work;
    replay.value(Value(1), 0, 0).unwrap();
    assert_eq!(replay.budget.work, old);
}
#[test]
fn scatter_work_rejects_before_loop_growth() {
    let ops = vec![
        (0, Op::Load(None)),
        (
            0,
            Op::Scatter {
                address: Value(0),
                data: Value(0),
                offset: 0,
                exec: u32::MAX,
            },
        ),
    ];
    let mut source = String::new();
    let mut budget = ReplayBudget {
        nodes: 0,
        work: 8191,
        ..ReplayBudget::default()
    };
    let mut replay = Replay::new(
        &mut source,
        &ops,
        &[false; 2],
        2,
        &mut budget,
        "lane".into(),
        "lane_index".into(),
        false,
    );
    assert!(
        replay
            .value(Value(1), 0, 0)
            .unwrap_err()
            .to_string()
            .contains("weighted work")
    );
    assert!(source.is_empty());
}
