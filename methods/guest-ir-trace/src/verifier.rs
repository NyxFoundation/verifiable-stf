use ir_trace_common::primitives::eval_primitive;
use ir_trace_common::trace_types::{Trace, TraceStep, TRACE_MAGIC};
use ir_trace_common::value::Value;

pub fn verify_trace(trace: &Trace) {
    // Defensive header checks
    assert_eq!(
        trace.header.magic, TRACE_MAGIC,
        "Invalid trace magic"
    );
    assert_eq!(
        trace.header.value_count as usize,
        trace.value_table.len(),
        "Header value_count ({}) != value_table.len() ({})",
        trace.header.value_count,
        trace.value_table.len()
    );
    assert_eq!(
        trace.header.step_count as usize,
        trace.steps.len(),
        "Header step_count ({}) != steps.len() ({})",
        trace.header.step_count,
        trace.steps.len()
    );
    let vlen = trace.value_table.len() as u32;
    assert!(
        trace.output_value_id < vlen,
        "output_value_id ({}) out of range (value_table has {} entries)",
        trace.output_value_id,
        vlen
    );

    let values = &trace.value_table;

    for (i, step) in trace.steps.iter().enumerate() {
        match step {
            TraceStep::PrimResult { op, args, result } => {
                assert!(*result < vlen, "PrimResult result id out of range at step {}", i);
                for a in args {
                    assert!(*a < vlen, "PrimResult arg id out of range at step {}", i);
                }
                let arg_values: Vec<Value> = args
                    .iter()
                    .map(|id| values[*id as usize].clone())
                    .collect();
                let computed = eval_primitive(op, &arg_values);
                assert_eq!(
                    computed,
                    values[*result as usize],
                    "Primitive verification failed at step {}: {:?}",
                    i,
                    op
                );
            }
            TraceStep::Branch {
                scrutinee,
                chosen_tag,
            } => {
                assert!(*scrutinee < vlen, "Branch scrutinee id out of range at step {}", i);
                let val = &values[*scrutinee as usize];
                assert_eq!(
                    val.tag(),
                    *chosen_tag,
                    "Branch verification failed at step {}: expected tag {}, got {}",
                    i,
                    chosen_tag,
                    val.tag()
                );
            }
            TraceStep::CtorCreate {
                tag,
                fields,
                scalar_data,
                result,
            } => {
                assert!(*result < vlen, "CtorCreate result id out of range at step {}", i);
                for f in fields {
                    assert!(*f < vlen, "CtorCreate field id out of range at step {}", i);
                }
                let obj = &values[*result as usize];
                assert_eq!(
                    obj.tag(),
                    *tag,
                    "Ctor tag mismatch at step {}",
                    i
                );
                match obj {
                    Value::Object {
                        fields: obj_fields,
                        scalars,
                        ..
                    } => {
                        assert_eq!(
                            obj_fields.len(),
                            fields.len(),
                            "Ctor field count mismatch at step {}",
                            i
                        );
                        for (j, field_id) in fields.iter().enumerate() {
                            assert_eq!(
                                obj_fields[j],
                                values[*field_id as usize],
                                "Ctor field {} mismatch at step {}",
                                j,
                                i
                            );
                        }
                        assert_eq!(
                            scalars, scalar_data,
                            "Ctor scalar content mismatch at step {}",
                            i
                        );
                    }
                    _ => panic!("Ctor result is not an Object at step {}", i),
                }
            }
            TraceStep::ProjResult { obj, idx, result } => {
                assert!(*obj < vlen, "ProjResult obj id out of range at step {}", i);
                assert!(*result < vlen, "ProjResult result id out of range at step {}", i);
                let obj_val = &values[*obj as usize];
                let expected = obj_val.field(*idx as usize);
                assert_eq!(
                    *expected,
                    values[*result as usize],
                    "Proj verification failed at step {}",
                    i
                );
            }
            TraceStep::SetResult {
                obj,
                idx,
                val,
                result,
            } => {
                assert!(*obj < vlen, "SetResult obj id out of range at step {}", i);
                assert!(*val < vlen, "SetResult val id out of range at step {}", i);
                assert!(*result < vlen, "SetResult result id out of range at step {}", i);
                let mut expected = values[*obj as usize].clone();
                expected.set_field(*idx as usize, values[*val as usize].clone());
                assert_eq!(
                    expected,
                    values[*result as usize],
                    "Set verification failed at step {}",
                    i
                );
            }
        }
    }
}
