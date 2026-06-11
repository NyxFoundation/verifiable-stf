use std::collections::{HashMap, HashSet};

use sha2::{Digest, Sha256};

use ir_trace_common::trace_types::{Trace, TraceHeader, TraceStep, ValueId, TRACE_MAGIC};
use ir_trace_common::value::Value;

use crate::interpreter::Interpreter;

pub fn build_trace(
    interpreter: &mut Interpreter,
    ir_program_bytes: &[u8],
    input_bytes: &[u8],
    output_value: &Value,
    output_value_id: u32,
) -> Trace {
    let ir_program_hash = sha256(ir_program_bytes);
    let input_hash = sha256(input_bytes);
    let output_bytes = output_value.serialize_to_bytes();
    let output_hash = sha256(&output_bytes);

    // Move ownership from interpreter to avoid double-buffering
    let value_table = std::mem::take(&mut interpreter.value_registry.table);
    let steps = std::mem::take(&mut interpreter.trace_steps);

    let Compacted {
        value_count,
        leaf_values,
        steps,
        output_value_id,
    } = compact_and_seed(value_table, steps, output_value_id);

    let header = TraceHeader {
        magic: TRACE_MAGIC,
        ir_program_hash,
        input_hash,
        output_hash,
        value_count,
        step_count: steps.len() as u64,
    };

    Trace {
        header,
        leaf_values,
        steps,
        output_value_id,
    }
}

struct Compacted {
    value_count: u32,
    leaf_values: Vec<(ValueId, Value)>,
    steps: Vec<TraceStep>,
    output_value_id: ValueId,
}

/// Prune unreferenced values, compact ValueIds into a contiguous range, then
/// determine the minimal set of seed (leaf) values the guest cannot rebuild by
/// re-executing the steps: any value consumed before it is produced (inputs,
/// extern results, constants), plus the output if it is never produced.
///
/// The full value table is dropped; only the seed values are kept. The guest
/// reconstructs everything else by re-executing `steps` in order.
fn compact_and_seed(
    value_table: Vec<Value>,
    mut steps: Vec<TraceStep>,
    output_value_id: ValueId,
) -> Compacted {
    // 1. Collect referenced ValueIds (for pruning).
    let mut referenced = HashSet::new();
    referenced.insert(output_value_id);
    for step in &steps {
        for_each_ref(step, |id| {
            referenced.insert(id);
        });
    }

    // 2. Compact ids over referenced values; drop the unreferenced rest.
    let mut old_to_new: HashMap<ValueId, ValueId> = HashMap::new();
    let mut new_table: Vec<Value> = Vec::new();
    for (old_id, value) in value_table.into_iter().enumerate() {
        let old_id = old_id as ValueId;
        if referenced.contains(&old_id) {
            old_to_new.insert(old_id, new_table.len() as ValueId);
            new_table.push(value);
        }
    }
    let value_count = new_table.len() as u32;

    // 3. Remap step + output ids into the compacted space.
    for step in &mut steps {
        remap_step(step, &old_to_new);
    }
    let output_value_id = old_to_new[&output_value_id];

    // 4. Simulate availability in step order. A consumed id that is not yet
    //    produced must be supplied as a leaf (this naturally covers inputs,
    //    extern results, constants, and any value reused before its producing
    //    step under value dedup).
    let mut available: HashSet<ValueId> = HashSet::new();
    let mut seed: HashSet<ValueId> = HashSet::new();
    for step in &steps {
        for_each_consumer(step, |id| {
            if available.insert(id) {
                seed.insert(id);
            }
        });
        for_each_producer(step, |id| {
            available.insert(id);
        });
    }
    if !available.contains(&output_value_id) {
        seed.insert(output_value_id);
    }

    // 5. Materialize leaf values in ascending-id order (deterministic).
    let leaf_values: Vec<(ValueId, Value)> = new_table
        .into_iter()
        .enumerate()
        .filter_map(|(id, v)| {
            let id = id as ValueId;
            if seed.contains(&id) {
                Some((id, v))
            } else {
                None
            }
        })
        .collect();

    Compacted {
        value_count,
        leaf_values,
        steps,
        output_value_id,
    }
}

/// Visit every ValueId a step references (consumers and producers).
fn for_each_ref<F: FnMut(ValueId)>(step: &TraceStep, mut f: F) {
    for_each_consumer(step, &mut f);
    for_each_producer(step, &mut f);
}

/// Visit the ValueIds a step reads (its inputs).
fn for_each_consumer<F: FnMut(ValueId)>(step: &TraceStep, mut f: F) {
    match step {
        TraceStep::PrimResult { args, .. } => args.iter().for_each(|&a| f(a)),
        TraceStep::Branch { scrutinee, .. } => f(*scrutinee),
        TraceStep::CtorCreate { fields, .. } => fields.iter().for_each(|&x| f(x)),
        TraceStep::ProjResult { obj, .. } => f(*obj),
        TraceStep::SetResult { obj, val, .. } => {
            f(*obj);
            f(*val);
        }
    }
}

/// Visit the ValueId a step writes (its result), if any.
fn for_each_producer<F: FnMut(ValueId)>(step: &TraceStep, mut f: F) {
    match step {
        TraceStep::PrimResult { result, .. } => f(*result),
        TraceStep::Branch { .. } => {}
        TraceStep::CtorCreate { result, .. } => f(*result),
        TraceStep::ProjResult { result, .. } => f(*result),
        TraceStep::SetResult { result, .. } => f(*result),
    }
}

fn remap_step(step: &mut TraceStep, map: &HashMap<ValueId, ValueId>) {
    match step {
        TraceStep::PrimResult { args, result, .. } => {
            for a in args.iter_mut() {
                *a = map[a];
            }
            *result = map[result];
        }
        TraceStep::Branch { scrutinee, .. } => *scrutinee = map[scrutinee],
        TraceStep::CtorCreate { fields, result, .. } => {
            for x in fields.iter_mut() {
                *x = map[x];
            }
            *result = map[result];
        }
        TraceStep::ProjResult { obj, result, .. } => {
            *obj = map[obj];
            *result = map[result];
        }
        TraceStep::SetResult {
            obj, val, result, ..
        } => {
            *obj = map[obj];
            *val = map[val];
            *result = map[result];
        }
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ir_trace_common::trace_types::TRACE_MAGIC;

    use super::*;

    #[test]
    fn compact_and_seed_prunes_remaps_and_keeps_only_leaves() {
        // value_table[1] (999) is unreferenced -> pruned. The two args (10, 20)
        // are consumed before being produced -> seeded as leaves. The result
        // (30) is produced by the step -> rebuilt by the guest, not seeded.
        let value_table = vec![
            Value::Scalar(10),
            Value::Scalar(999),
            Value::Scalar(20),
            Value::Scalar(30),
        ];
        let steps = vec![TraceStep::PrimResult {
            op: ir_trace_common::trace_types::PrimOp::UInt64Add,
            args: vec![0, 2],
            result: 3,
        }];

        let compacted = compact_and_seed(value_table, steps, 3);

        assert_eq!(compacted.value_count, 3);
        assert_eq!(
            compacted.leaf_values,
            vec![(0, Value::Scalar(10)), (1, Value::Scalar(20))]
        );
        assert_eq!(compacted.output_value_id, 2);

        match &compacted.steps[0] {
            TraceStep::PrimResult { args, result, .. } => {
                assert_eq!(args, &vec![0, 1]);
                assert_eq!(*result, 2);
            }
            step => panic!("unexpected step after compaction: {:?}", step),
        }
    }

    #[test]
    fn build_trace_emits_new_magic_seeds_and_compacted_counts() {
        let mut interpreter = Interpreter::new(HashMap::new());
        interpreter.value_registry.table =
            vec![Value::Scalar(1), Value::Scalar(777), Value::Scalar(2)];
        interpreter.trace_steps = vec![TraceStep::PrimResult {
            op: ir_trace_common::trace_types::PrimOp::UInt64Add,
            args: vec![0],
            result: 2,
        }];

        let trace = build_trace(
            &mut interpreter,
            b"dummy-ir",
            b"dummy-input",
            &Value::Scalar(2),
            2,
        );

        assert_eq!(trace.header.magic, TRACE_MAGIC);
        assert_eq!(trace.header.value_count, 2);
        assert_eq!(trace.header.step_count, 1);
        // Only the consumed-before-produced arg (1) is seeded; the result (2)
        // is rebuilt by the guest.
        assert_eq!(trace.leaf_values, vec![(0, Value::Scalar(1))]);
        assert_eq!(trace.output_value_id, 1);
        assert!(interpreter.value_registry.table.is_empty());
        assert!(interpreter.trace_steps.is_empty());
    }
}
