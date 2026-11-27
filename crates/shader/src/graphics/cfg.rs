// SPDX-License-Identifier: GPL-3.0-or-later
//! Structural proofs for nested, pretested unsigned and signed-positive countdowns.
use super::*;

pub(super) struct CountedLoop {
    pub header: usize,
    pub latch: usize,
    pub counter: u8,
    pub signed_positive: bool,
    pub parent: Option<usize>,
    decrement: usize,
    nonincreasing_scalars: [bool; 106],
    signed_nonincreasing_scalars: [bool; 106],
    pub scalar_writes: [bool; 106],
    pub vector_writes: [bool; 256],
}

fn reject(offset: usize, reason: &'static str) -> CompileError {
    CompileError::Verify { offset, reason }
}

pub(super) fn analyze(decoded: &[DecodedInstruction]) -> Result<Vec<CountedLoop>, CompileError> {
    if decoded.len() > 1024
        && decoded
            .iter()
            .any(|i| i.branch_target().is_some_and(|t| t <= i.offset))
    {
        return Err(reject(0, "graphics loop analysis limit exceeded"));
    }
    let target_index = |target| {
        decoded
            .binary_search_by_key(&target, |i| i.offset)
            .map_err(|_| reject(target, "branch target is not an instruction boundary"))
    };
    let mut edges = vec![Vec::new(); decoded.len()];
    for (i, inst) in decoded.iter().enumerate() {
        if let Some(target) = inst.branch_target() {
            let destination = target_index(target)?;
            if matches!(
                decoded[destination].instruction,
                Instruction::Interpolate {
                    phase: InterpolationPhase::Second,
                    ..
                }
            ) {
                return Err(reject(
                    inst.offset,
                    "branch target enters the second interpolation phase",
                ));
            }
            if target <= inst.offset && !matches!(inst.instruction, Instruction::Branch { .. }) {
                return Err(reject(
                    inst.offset,
                    "counted loop requires an unconditional latch",
                ));
            }
            edges[i].push(destination);
        }
        if !matches!(
            inst.instruction,
            Instruction::Branch { .. } | Instruction::EndProgram
        ) {
            let next = i + if matches!(
                inst.instruction,
                Instruction::Interpolate {
                    phase: InterpolationPhase::First,
                    ..
                }
            ) {
                2
            } else {
                1
            };
            if next < decoded.len() {
                edges[i].push(next);
            }
        }
    }
    let mut loops: Vec<CountedLoop> = Vec::new();
    for (latch, inst) in decoded.iter().enumerate() {
        let Instruction::Branch { target } = inst.instruction else {
            continue;
        };
        if target > inst.offset {
            continue;
        }
        let header = target_index(target)?;
        if header == 0 || header + 2 >= latch {
            return Err(reject(
                inst.offset,
                "counted loop requires a preheader and nonempty body",
            ));
        }
        let (counter, signed_positive) = match decoded[header].instruction {
            Instruction::ScalarCompare {
                op: ScalarCompareOp::CmpLgU32,
                src0: ScalarSource::Register(r),
                src1: ScalarSource::Immediate(0),
            }
            | Instruction::ScalarCompare {
                op: ScalarCompareOp::CmpLgU32,
                src1: ScalarSource::Register(r),
                src0: ScalarSource::Immediate(0),
            } if (32..106).contains(&r) => (r, false),
            Instruction::ScalarCompare {
                op: ScalarCompareOp::CmpGtI32,
                src0: ScalarSource::Register(r),
                src1: ScalarSource::Immediate(0),
            }
            | Instruction::ScalarCompare {
                op: ScalarCompareOp::CmpLtI32,
                src0: ScalarSource::Immediate(0),
                src1: ScalarSource::Register(r),
            } if (32..106).contains(&r) => (r, true),
            _ => {
                return Err(reject(
                    decoded[header].offset,
                    "loop header requires unsigned counter != 0 or signed counter > 0",
                ));
            }
        };
        let exit = match decoded[header + 1].instruction {
            Instruction::BranchSccZero { target } => target_index(target)?,
            _ => {
                return Err(reject(
                    decoded[header + 1].offset,
                    "loop header requires adjacent SCC0 exit",
                ));
            }
        };
        if exit != latch + 1 {
            return Err(reject(
                inst.offset,
                "loop requires a single contiguous exit",
            ));
        }
        let mut scalar_writes = [false; 106];
        let mut vector_writes = [false; 256];
        let mut decrement = None;
        for (i, item) in decoded.iter().enumerate().take(latch).skip(header + 2) {
            match item.instruction {
                Instruction::ScalarMoveB32 { dst, .. }
                | Instruction::ScalarAlu { dst, .. }
                | Instruction::ScalarBitSetB32 { dst, .. }
                | Instruction::ScalarConditionalMoveB32 { dst, .. }
                | Instruction::ScalarConditionalMoveImmediateI32 { dst, .. } => {
                    if usize::from(dst) < scalar_writes.len() {
                        scalar_writes[usize::from(dst)] = true;
                    }
                    if dst == counter {
                        if decrement.is_some()
                            || !matches!(item.instruction, Instruction::ScalarAlu {
                            op, src0: ScalarSource::Register(r), src1: ScalarSource::Immediate(1), ..
                        } if r == counter && (op == ScalarAluOp::SubU32
                            || (signed_positive && op == ScalarAluOp::SubI32)))
                        {
                            return Err(reject(
                                item.offset,
                                "loop counter requires exactly one current-value decrement",
                            ));
                        }
                        decrement = Some(i);
                    }
                }
                Instruction::MoveB32 { dst, .. }
                | Instruction::VectorAddF32 { dst, .. }
                | Instruction::VectorMulF32 { dst, .. }
                | Instruction::Interpolate { dst, .. } => vector_writes[usize::from(dst)] = true,
                Instruction::ImageSampleLz { dst, .. }
                | Instruction::ImageSampleLzArray { dst, .. } => {
                    for written in vector_writes.iter_mut().skip(usize::from(dst)).take(4) {
                        *written = true;
                    }
                }
                Instruction::ImageStore2D { .. } => {} // Ordered effect, no register writes.
                Instruction::Export { .. } | Instruction::EndProgram => {
                    return Err(reject(
                        item.offset,
                        "exports and termination cannot occur in a loop",
                    ));
                }
                _ => {}
            }
        }
        let decrement =
            decrement.ok_or_else(|| reject(inst.offset, "loop counter has no decrement"))?;
        // Removing the update must disconnect the body entry from the latch.
        let mut seen = vec![false; decoded.len()];
        let mut pending = vec![header + 2];
        while let Some(node) = pending.pop() {
            if node == decrement || node == exit || seen[node] {
                continue;
            }
            if node == latch {
                return Err(reject(
                    inst.offset,
                    "loop decrement does not dominate latch",
                ));
            }
            seen[node] = true;
            pending.extend(edges[node].iter().copied());
        }
        loops.push(CountedLoop {
            header,
            latch,
            counter,
            signed_positive,
            parent: None,
            decrement,
            nonincreasing_scalars: [false; 106],
            signed_nonincreasing_scalars: [false; 106],
            scalar_writes,
            vector_writes,
        });
    }
    if !loops.is_empty() && (decoded.len() > 1024 || loops.len() > 8) {
        return Err(reject(0, "graphics loop analysis limit exceeded"));
    }
    // Discover every interval before assigning parents: latch order visits a
    // child before its parent. Every backward edge already has a recognized
    // latch; deleting those edges leaves only forward edges.
    for a in 0..loops.len() {
        for b in a + 1..loops.len() {
            let left = &loops[a];
            let right = &loops[b];
            let overlap = left.header <= right.latch && right.header <= left.latch;
            let contains = |outer: &CountedLoop, inner: &CountedLoop| {
                outer.header < inner.header && inner.latch < outer.latch
            };
            if overlap && !contains(left, right) && !contains(right, left) {
                return Err(reject(
                    decoded[right.latch].offset,
                    "counted loop intervals overlap without strict nesting",
                ));
            }
        }
    }
    for child in 0..loops.len() {
        loops[child].parent = (0..loops.len())
            .filter(|&parent| {
                loops[parent].header < loops[child].header
                    && loops[child].latch < loops[parent].latch
            })
            .min_by_key(|&parent| loops[parent].latch - loops[parent].header);
    }
    // Classify exits only after the complete loop forest is known. An inner
    // body may break to its own continuation, never across an ancestor boundary.
    for info in &loops {
        for (from, successors) in edges.iter().enumerate() {
            for &to in successors {
                if !(info.header..=info.latch).contains(&from)
                    && (info.header + 1..=info.latch).contains(&to)
                {
                    return Err(reject(decoded[from].offset, "loop has an interior entry"));
                }
                if (info.header + 2..info.latch).contains(&from)
                    && !(info.header + 2..=info.latch).contains(&to)
                {
                    let innermost = loops
                        .iter()
                        .filter(|candidate| (candidate.header..=candidate.latch).contains(&from))
                        .min_by_key(|candidate| candidate.latch - candidate.header)
                        .expect("body belongs to a loop");
                    if to != info.latch + 1 || innermost.header != info.header {
                        return Err(reject(
                            decoded[from].offset,
                            "loop body has an unsupported exit or cycle",
                        ));
                    }
                }
            }
        }
    }
    for outer in &loops {
        for child in &loops {
            if outer.header < child.header && child.latch < outer.latch {
                if outer.counter == child.counter {
                    return Err(reject(
                        decoded[child.header].offset,
                        "nested loops must use distinct counters",
                    ));
                }
                if (child.header..=child.latch).contains(&outer.decrement) {
                    return Err(reject(
                        decoded[outer.decrement].offset,
                        "ancestor counter decrement cannot execute inside a child loop",
                    ));
                }
            }
        }
    }
    for index in 0..loops.len() {
        let mut nonincreasing = [true; 106];
        let mut signed_nonincreasing = [true; 106];
        for (instruction, decoded_instruction) in decoded
            .iter()
            .enumerate()
            .take(loops[index].latch)
            .skip(loops[index].header + 2)
        {
            if let Instruction::ScalarMoveB32 { dst, .. }
            | Instruction::ScalarAlu { dst, .. }
            | Instruction::ScalarBitSetB32 { dst, .. }
            | Instruction::ScalarConditionalMoveB32 { dst, .. }
            | Instruction::ScalarConditionalMoveImmediateI32 { dst, .. } =
                decoded_instruction.instruction
                && usize::from(dst) < nonincreasing.len()
            {
                let update = loops
                    .iter()
                    .find(|info| info.decrement == instruction && info.counter == dst);
                // Every verified guard excludes unsigned zero before decrement.
                // Only a signed-positive guard also excludes INT_MIN: unsigned
                // !=0 alone permits INT_MIN -> INT_MAX, increasing signed order.
                if update.is_none() {
                    nonincreasing[usize::from(dst)] = false;
                }
                if !update.is_some_and(|info| info.signed_positive) {
                    signed_nonincreasing[usize::from(dst)] = false;
                }
            }
        }
        loops[index].nonincreasing_scalars = nonincreasing;
        loops[index].signed_nonincreasing_scalars = signed_nonincreasing;
    }
    Ok(loops)
}

/// Bound each decoded instruction, including both interpolation phases and the
/// final failed header test on every invocation of a child loop.
pub(super) fn validate_work(
    instruction_count: usize,
    loops: &[CountedLoop],
    bounds: &[Option<u32>],
    reachable: &[bool],
) -> Result<(), CompileError> {
    for (index, info) in loops.iter().enumerate() {
        if !reachable[index] || bounds[index].is_some() {
            continue;
        }
        let mut parent = info.parent;
        let mut under_zero = false;
        while let Some(ancestor) = parent {
            if bounds[ancestor] == Some(0) {
                under_zero = true;
                break;
            }
            parent = loops[ancestor].parent;
        }
        if !under_zero {
            return Err(reject(
                0,
                "unreachable loop lacks an independent entry bound",
            ));
        }
    }
    let mut total = 0usize;
    for instruction in 0..instruction_count {
        // Entire absent intervals cannot execute: full structural analysis
        // excludes interior entry, and only constant-selected edges are pruned.
        if loops.iter().enumerate().any(|(index, info)| {
            !reachable[index] && (info.header..=info.latch).contains(&instruction)
        }) {
            continue;
        }
        // Check zero ancestors before requesting descendant bounds or multiplying.
        if loops.iter().enumerate().any(|(index, info)| {
            (info.header + 2..=info.latch).contains(&instruction) && bounds[index] == Some(0)
        }) {
            continue;
        }
        let mut cost = 1usize;
        for (index, info) in loops.iter().enumerate() {
            if !(info.header..=info.latch).contains(&instruction) {
                continue;
            }
            let bound = bounds[index]
                .ok_or_else(|| reject(0, "unreachable loop lacks an independent entry bound"))?
                as usize;
            let factor = bound + usize::from(instruction < info.header + 2);
            cost = cost
                .checked_mul(factor)
                .filter(|cost| *cost <= 65536)
                .ok_or_else(|| reject(0, "graphics loop work limit exceeded"))?;
        }
        total = total
            .checked_add(cost)
            .filter(|total| *total <= 65536)
            .ok_or_else(|| reject(0, "graphics loop work limit exceeded"))?;
    }
    Ok(())
}

// Include remaining depth in the key: an exhausted/partial answer at one
// depth must not suppress a more precise answer reached along a shorter path.
// There are at most values.len() * fuel entries per domain, including unknowns.
// Each analysis borrows immutable IR, so subsequently added phi facts cannot
// invalidate cached answers.
struct BoundAnalysis<'a> {
    ir: &'a StageIr,
    unsigned: std::collections::HashMap<(usize, usize), Option<u32>>,
    signed: std::collections::HashMap<(usize, usize), Option<i32>>,
    #[cfg(test)]
    visits: usize,
}

impl<'a> BoundAnalysis<'a> {
    fn new(ir: &'a StageIr) -> Self {
        Self {
            ir,
            unsigned: Default::default(),
            signed: Default::default(),
            #[cfg(test)]
            visits: 0,
        }
    }

    fn unsigned(&mut self, value: ScalarValue, fuel: usize) -> Option<u32> {
        if fuel == 0 {
            return None;
        }
        let value = match value {
            ScalarValue::Constant(value) => return Some(value),
            ScalarValue::Uniform(_) => return None,
            ScalarValue::Runtime(value) => value,
        };
        let key = (value.0, fuel);
        if let Some(bound) = self.unsigned.get(&key) {
            return *bound;
        }
        #[cfg(test)]
        {
            self.visits += 1;
        }
        let bound = if let Some((_, bound)) = self
            .ir
            .scalar_upper_bounds
            .iter()
            .find(|(known, _)| known.0 == value.0)
        {
            Some(*bound)
        } else if let Op::ScalarResult(ScalarAluOp::MinU32, left, right, _) =
            self.ir.values[value.0].1
        {
            match (
                self.unsigned(left, fuel - 1),
                self.unsigned(right, fuel - 1),
            ) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) | (None, Some(a)) => Some(a),
                _ => None,
            }
        } else {
            None
        };
        self.unsigned.insert(key, bound);
        bound
    }

    fn signed(&mut self, value: ScalarValue, fuel: usize) -> Option<i32> {
        if fuel == 0 {
            return None;
        }
        let value = match value {
            ScalarValue::Constant(value) => return Some(value as i32),
            ScalarValue::Uniform(_) => return None,
            ScalarValue::Runtime(value) => value,
        };
        let key = (value.0, fuel);
        if let Some(bound) = self.signed.get(&key) {
            return *bound;
        }
        #[cfg(test)]
        {
            self.visits += 1;
        }
        let bound = if let Some((_, bound)) = self
            .ir
            .signed_scalar_upper_bounds
            .iter()
            .find(|(known, _)| known.0 == value.0)
        {
            Some(*bound)
        } else if let Op::ScalarResult(ScalarAluOp::MinI32, left, right, _) =
            self.ir.values[value.0].1
        {
            match (self.signed(left, fuel - 1), self.signed(right, fuel - 1)) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) | (None, Some(a)) => Some(a),
                _ => None,
            }
        } else {
            // Only unsigned facts below the sign bit convert to signed facts.
            // The cross-domain query shares this analysis and its caches.
            self.unsigned(ScalarValue::Runtime(value), fuel - 1)
                .and_then(|bound| i32::try_from(bound).ok())
        };
        self.signed.insert(key, bound);
        bound
    }
}

pub(super) fn upper_bound(value: ScalarValue, ir: &StageIr, fuel: usize) -> Option<u32> {
    BoundAnalysis::new(ir).unsigned(value, fuel)
}

/// A signed value fact, never an unsigned bound or an iteration count.
pub(super) fn signed_upper_bound(value: ScalarValue, ir: &StageIr, fuel: usize) -> Option<i32> {
    BoundAnalysis::new(ir).signed(value, fuel)
}

pub(super) fn trip_bound(info: &CountedLoop, value: ScalarValue, ir: &StageIr) -> Option<u32> {
    if info.signed_positive {
        signed_upper_bound(value, ir, 1024).map(|bound| bound.max(0) as u32)
    } else {
        upper_bound(value, ir, 1024)
    }
}

pub(super) fn header_state(
    ir: &mut StageIr,
    state: &RegisterState,
    info: &CountedLoop,
    offset: usize,
) -> RegisterState {
    let mut header = state.clone();
    for r in 0..256 {
        if info.vector_writes[r] && state.registers[r].is_some() {
            header.registers[r] = Some(ir.push(Some(offset), Op::Phi(ValueType::F32)));
        }
    }
    for r in 32..106 {
        if info.scalar_writes[r] && state.scalars[r].is_some() {
            let bound = if info.nonincreasing_scalars[r] {
                upper_bound(state.scalars[r].expect("initialized scalar"), ir, 1024)
            } else {
                None
            };
            let signed_bound = if info.signed_nonincreasing_scalars[r] {
                signed_upper_bound(state.scalars[r].expect("initialized scalar"), ir, 1024)
            } else {
                None
            };
            let phi = ir.push(Some(offset), Op::Phi(ValueType::U32));
            if let Some(bound) = bound {
                // All writes are structurally verified positive countdown steps.
                // They cannot increase this independent preheader bound or wrap.
                ir.scalar_upper_bounds.push((phi, bound));
            }
            if let Some(bound) = signed_bound {
                ir.signed_scalar_upper_bounds.push((phi, bound));
            }
            header.scalars[r] = Some(ScalarValue::Runtime(phi));
        }
    }
    // Header compare always overwrites SCC before any body/exit use. Its real
    // final evaluation, including zero-trip execution, supplies the live-out flag.
    header
}

pub(super) fn copies(
    header: &RegisterState,
    source: &RegisterState,
    info: &CountedLoop,
) -> Vec<(Value, EdgeValue)> {
    let mut copies = Vec::new();
    for r in 0..256 {
        if info.vector_writes[r]
            && let Some(dst) = header.registers[r]
        {
            copies.push((
                dst,
                EdgeValue::Float(source.registers[r].expect("entry definition preserved")),
            ));
        }
    }
    for r in 32..106 {
        if info.scalar_writes[r]
            && let Some(ScalarValue::Runtime(dst)) = header.scalars[r]
        {
            copies.push((
                dst,
                EdgeValue::Uint(source.scalars[r].expect("entry definition preserved")),
            ));
        }
    }
    copies
}

#[cfg(test)]
mod signed_fact_tests {
    use super::*;

    #[test]
    fn unsigned_decrement_drops_signed_fact_before_any_trip_admission() {
        for signed_positive in [false, true] {
            // Analyze structure only: this directly tests the fact-transfer
            // boundary, without an earlier bound rejection hiding the result.
            let words = [
                0xbea00318u32,
                if signed_positive {
                    0xbf028020
                } else {
                    0xbf078020
                },
                0xbf840002,
                0x80a08120,
                0xbf82fffc,
                0xbf810000,
            ];
            let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
            let decoded = crate::decode_fixture(Architecture::Gfx1030, &bytes).unwrap();
            let loops = analyze(&decoded).unwrap();
            let mut ir = StageIr::default();
            let value = ir.push(
                None,
                Op::ScalarResult(
                    ScalarAluOp::MinI32,
                    ScalarValue::Uniform(0),
                    ScalarValue::Constant(u32::MAX),
                    Condition::Constant(false),
                ),
            );
            let mut state = RegisterState {
                registers: [None; 256],
                scalars: [None; 106],
                scc: None,
                last_export: None,
                pixel_done: false,
            };
            state.scalars[32] = Some(ScalarValue::Runtime(value));
            assert_eq!(
                signed_upper_bound(state.scalars[32].unwrap(), &ir, 1024),
                Some(-1)
            );
            let header = header_state(&mut ir, &state, &loops[0], 4);
            let carried = header.scalars[32].unwrap();
            assert_eq!(upper_bound(carried, &ir, 1024), None);
            assert_eq!(
                signed_upper_bound(carried, &ir, 1024),
                if signed_positive { Some(-1) } else { None }
            );
        }
    }
}

#[cfg(test)]
mod bound_dag_tests {
    use super::*;

    fn min(ir: &mut StageIr, op: ScalarAluOp, a: ScalarValue, b: ScalarValue) -> ScalarValue {
        ScalarValue::Runtime(ir.push(None, Op::ScalarResult(op, a, b, Condition::Constant(false))))
    }

    #[test]
    fn shared_known_and_unknown_diamonds_visit_each_depth_key_once() {
        for signed in [false, true] {
            for known in [false, true] {
                let mut ir = StageIr::default();
                let op = if signed {
                    ScalarAluOp::MinI32
                } else {
                    ScalarAluOp::MinU32
                };
                let mut root = min(
                    &mut ir,
                    op,
                    ScalarValue::Uniform(0),
                    if known {
                        ScalarValue::Constant(if signed { u32::MAX } else { 7 })
                    } else {
                        ScalarValue::Uniform(1)
                    },
                );
                // Distinct left/right identities prevent an identical-operand
                // shortcut from making this test accidentally linear.
                for _ in 0..60 {
                    let left = min(&mut ir, op, root, ScalarValue::Uniform(2));
                    let right = min(&mut ir, op, root, ScalarValue::Uniform(3));
                    root = min(&mut ir, op, left, right);
                }
                let mut analysis = BoundAnalysis::new(&ir);
                if signed {
                    assert_eq!(analysis.signed(root, 1024), known.then_some(-1));
                } else {
                    assert_eq!(analysis.unsigned(root, 1024), known.then_some(7));
                }
                assert_eq!(analysis.visits, ir.values.len());
                let visits = analysis.visits;
                if signed {
                    analysis.signed(root, 1024);
                } else {
                    analysis.unsigned(root, 1024);
                }
                assert_eq!(
                    analysis.visits, visits,
                    "known and unknown roots are cached"
                );
            }
        }
    }

    #[test]
    fn remaining_depth_is_part_of_the_cache_key_in_both_domains() {
        for signed in [false, true] {
            let mut ir = StageIr::default();
            let op = if signed {
                ScalarAluOp::MinI32
            } else {
                ScalarAluOp::MinU32
            };
            let leaf = min(
                &mut ir,
                op,
                ScalarValue::Uniform(0),
                ScalarValue::Constant(7),
            );
            let root = min(&mut ir, op, leaf, ScalarValue::Uniform(1));
            let mut analysis = BoundAnalysis::new(&ir);
            if signed {
                assert_eq!(analysis.signed(root, 2), None);
                assert_eq!(analysis.signed(root, 3), Some(7));
                assert_eq!(analysis.signed(root, 2), None);
            } else {
                assert_eq!(analysis.unsigned(root, 2), None);
                assert_eq!(analysis.unsigned(root, 3), Some(7));
                assert_eq!(analysis.unsigned(root, 2), None);
            }
            assert_eq!(analysis.visits, 4);
        }
    }

    #[test]
    fn signed_fallback_shares_unsigned_cache_and_preserves_depth() {
        let mut ir = StageIr::default();
        let mut root = min(
            &mut ir,
            ScalarAluOp::MinU32,
            ScalarValue::Uniform(0),
            ScalarValue::Constant(7),
        );
        for _ in 0..60 {
            root = min(&mut ir, ScalarAluOp::MinU32, root, root);
        }
        let mut analysis = BoundAnalysis::new(&ir);
        assert_eq!(analysis.signed(root, 1024), Some(7));
        assert_eq!(analysis.visits, ir.values.len() + 1);
        let visits = analysis.visits;
        assert_eq!(analysis.unsigned(root, 1023), Some(7));
        assert_eq!(analysis.visits, visits);
    }
}
