use ir_trace_common::primitives::eval_primitive;
use ir_trace_common::trace_types::{Trace, TraceStep, TRACE_MAGIC};
use ir_trace_common::value::Value;

/// Reconstruct the full value table by re-executing the trace steps in order.
///
/// The trace ships only the seed (leaf) values the guest cannot recompute —
/// inputs, extern (crypto stub) results, and literal constants. Every other
/// value is produced here by replaying the recorded steps, so the host never
/// has to serialize the full value table.
///
/// Returns the reconstructed value table, indexed by ValueId.
pub fn execute_trace(trace: &Trace) -> Vec<Value> {
    assert_eq!(trace.header.magic, TRACE_MAGIC, "Invalid trace magic");

    let value_count = trace.header.value_count as usize;
    let mut values: Vec<Value> = vec![Value::Irrelevant; value_count];

    // Seed the leaf values the guest cannot derive from steps alone.
    for (id, value) in &trace.leaf_values {
        let id = *id as usize;
        assert!(
            id < value_count,
            "Leaf id {} out of range (value_count {})",
            id,
            value_count
        );
        values[id] = value.clone();
    }

    // Replay steps, filling in each produced value.
    for (i, step) in trace.steps.iter().enumerate() {
        match step {
            TraceStep::PrimResult { op, args, result } => {
                let arg_values: Vec<Value> =
                    args.iter().map(|id| values[*id as usize].clone()).collect();
                values[*result as usize] = eval_primitive(op, &arg_values);
            }
            TraceStep::Branch {
                scrutinee,
                chosen_tag,
            } => {
                // Control-flow consistency: the reconstructed scrutinee must
                // carry the tag the host branched on.
                let tag = values[*scrutinee as usize].tag();
                assert_eq!(
                    tag, *chosen_tag,
                    "Branch tag mismatch at step {}: expected {}, got {}",
                    i, chosen_tag, tag
                );
            }
            TraceStep::CtorCreate {
                tag,
                fields,
                scalar_data,
                result,
            } => {
                let field_values: Vec<Value> =
                    fields.iter().map(|id| values[*id as usize].clone()).collect();
                values[*result as usize] = Value::Object {
                    tag: *tag,
                    fields: field_values,
                    scalars: scalar_data.clone(),
                };
            }
            TraceStep::ProjResult { obj, idx, result } => {
                let projected = values[*obj as usize].field(*idx as usize).clone();
                values[*result as usize] = projected;
            }
            TraceStep::SetResult {
                obj,
                idx,
                val,
                result,
            } => {
                let mut new_obj = values[*obj as usize].clone();
                new_obj.set_field(*idx as usize, values[*val as usize].clone());
                values[*result as usize] = new_obj;
            }
        }
    }

    values
}
