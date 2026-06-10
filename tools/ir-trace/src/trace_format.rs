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

    let header = TraceHeader {
        magic: TRACE_MAGIC,
        ir_program_hash,
        input_hash,
        output_hash,
        value_count: value_table.len() as u32,
        step_count: steps.len() as u64,
    };

    let mut trace = Trace {
        header,
        value_table,
        steps,
        output_value_id,
    };

    compact_trace(&mut trace);
    trace
}

/// Remove unreferenced values and compact ValueIds.
fn compact_trace(trace: &mut Trace) {
    // Collect all referenced ValueIds
    let mut referenced = HashSet::new();
    referenced.insert(trace.output_value_id);

    for step in &trace.steps {
        match step {
            TraceStep::PrimResult { args, result, .. } => {
                referenced.extend(args);
                referenced.insert(*result);
            }
            TraceStep::Branch { scrutinee, .. } => {
                referenced.insert(*scrutinee);
            }
            TraceStep::CtorCreate { fields, result, .. } => {
                referenced.extend(fields);
                referenced.insert(*result);
            }
            TraceStep::ProjResult { obj, result, .. } => {
                referenced.insert(*obj);
                referenced.insert(*result);
            }
            TraceStep::SetResult { obj, val, result, .. } => {
                referenced.insert(*obj);
                referenced.insert(*val);
                referenced.insert(*result);
            }
        }
    }

    // Build compaction map: old_id -> new_id
    let mut old_to_new: HashMap<ValueId, ValueId> = HashMap::new();
    let mut new_table = Vec::new();
    let old_table = std::mem::take(&mut trace.value_table);
    for (old_id, value) in old_table.into_iter().enumerate() {
        let old_id = old_id as ValueId;
        if referenced.contains(&old_id) {
            old_to_new.insert(old_id, new_table.len() as ValueId);
            new_table.push(value);
        }
    }

    // Rewrite all ValueId references
    let remap = |id: &mut ValueId| {
        *id = old_to_new[id];
    };

    for step in &mut trace.steps {
        match step {
            TraceStep::PrimResult { args, result, .. } => {
                for a in args.iter_mut() {
                    remap(a);
                }
                remap(result);
            }
            TraceStep::Branch { scrutinee, .. } => {
                remap(scrutinee);
            }
            TraceStep::CtorCreate { fields, result, .. } => {
                for f in fields.iter_mut() {
                    remap(f);
                }
                remap(result);
            }
            TraceStep::ProjResult { obj, result, .. } => {
                remap(obj);
                remap(result);
            }
            TraceStep::SetResult { obj, val, result, .. } => {
                remap(obj);
                remap(val);
                remap(result);
            }
        }
    }

    trace.output_value_id = old_to_new[&trace.output_value_id];
    trace.value_table = new_table;
    trace.header.value_count = trace.value_table.len() as u32;
    trace.header.step_count = trace.steps.len() as u64;
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
    fn compact_trace_prunes_unreferenced_values_and_remaps_ids() {
        let mut trace = Trace {
            header: TraceHeader {
                magic: TRACE_MAGIC,
                ir_program_hash: [0; 32],
                input_hash: [0; 32],
                output_hash: [0; 32],
                value_count: 4,
                step_count: 1,
            },
            value_table: vec![
                Value::Scalar(10),
                Value::Scalar(999),
                Value::Scalar(20),
                Value::Scalar(30),
            ],
            steps: vec![TraceStep::PrimResult {
                op: ir_trace_common::trace_types::PrimOp::UInt64Add,
                args: vec![0, 2],
                result: 3,
            }],
            output_value_id: 3,
        };

        compact_trace(&mut trace);

        assert_eq!(
            trace.value_table,
            vec![Value::Scalar(10), Value::Scalar(20), Value::Scalar(30)]
        );
        assert_eq!(trace.header.value_count, 3);
        assert_eq!(trace.header.step_count, 1);
        assert_eq!(trace.output_value_id, 2);

        match &trace.steps[0] {
            TraceStep::PrimResult { args, result, .. } => {
                assert_eq!(args, &vec![0, 1]);
                assert_eq!(*result, 2);
            }
            step => panic!("unexpected step after compaction: {:?}", step),
        }
    }

    #[test]
    fn build_trace_emits_new_magic_and_compacted_counts() {
        let mut interpreter = Interpreter::new(HashMap::new());
        interpreter.value_registry.table = vec![
            Value::Scalar(1),
            Value::Scalar(777),
            Value::Scalar(2),
        ];
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
        assert_eq!(trace.value_table, vec![Value::Scalar(1), Value::Scalar(2)]);
        assert_eq!(trace.output_value_id, 1);
        assert!(interpreter.value_registry.table.is_empty());
        assert!(interpreter.trace_steps.is_empty());
    }
}
