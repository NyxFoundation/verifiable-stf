use ir_trace_common::trace_types::{TraceStep, ValueId};

use crate::ir_types::{Arg, Expr, IRType, LitValue};
use super::stack::CallFrame;
use super::value::Value;
use super::ValueRegistry;

pub struct ExprEvaluator<'a> {
    pub frame: &'a mut CallFrame,
    pub trace_steps: &'a mut Vec<TraceStep>,
    pub value_registry: &'a mut ValueRegistry,
}

impl<'a> ExprEvaluator<'a> {
    pub fn resolve_arg(&self, arg: &Arg) -> Value {
        match arg {
            Arg::Var(v) => self.frame.get_var(*v).clone(),
            Arg::Irrelevant => Value::Irrelevant,
        }
    }

    pub fn eval_with_ty(&mut self, expr: &Expr, ty: &IRType) -> Value {
        // For SProj, use the VDecl's type instead of the `n` parameter
        if let Expr::SProj { n: _, offset, var } = expr {
            let obj = self.frame.get_var(*var).clone();
            let size = ir_type_size(ty);
            let bytes = obj.get_scalar_bytes(0, *offset, size);
            let mut buf = [0u8; 8];
            let copy_len = size.min(bytes.len()).min(8);
            buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
            return Value::Scalar(u64::from_le_bytes(buf));
        }
        self.eval(expr)
    }

    pub fn eval(&mut self, expr: &Expr) -> Value {
        match expr {
            Expr::Ctor { info, args } => {
                let fields: Vec<Value> = args.iter().map(|a| self.resolve_arg(a)).collect();
                let scalars = vec![0u8; info.scalar_size as usize];
                let val = Value::Object {
                    tag: info.cidx,
                    fields,
                    scalars,
                };
                let field_ids: Vec<ValueId> = match &val {
                    Value::Object { fields, .. } => {
                        fields.iter().map(|f| self.value_registry.register(f)).collect()
                    }
                    _ => vec![],
                };
                let result_id = self.value_registry.register(&val);
                self.trace_steps.push(TraceStep::CtorCreate {
                    tag: info.cidx,
                    fields: field_ids,
                    scalar_data: match &val {
                        Value::Object { scalars, .. } => scalars.clone(),
                        _ => vec![],
                    },
                    result: result_id,
                });
                val
            }
            Expr::Proj { idx, var } => {
                let obj = self.frame.get_var(*var).clone();
                let field = obj.field(*idx as usize).clone();
                let obj_id = self.value_registry.register(&obj);
                let result_id = self.value_registry.register(&field);
                self.trace_steps.push(TraceStep::ProjResult {
                    obj: obj_id,
                    idx: *idx as u16,
                    result: result_id,
                });
                field
            }
            Expr::UProj { idx, var } => {
                // USize field projection - stored after regular fields
                let obj = self.frame.get_var(*var).clone();
                match &obj {
                    Value::Object { scalars, .. } => {
                        let offset = *idx as usize * std::mem::size_of::<usize>();
                        if offset + 8 <= scalars.len() {
                            let mut bytes = [0u8; 8];
                            bytes.copy_from_slice(&scalars[offset..offset + 8]);
                            Value::Scalar(u64::from_le_bytes(bytes))
                        } else {
                            Value::Scalar(0)
                        }
                    }
                    _ => Value::Scalar(0),
                }
            }
            Expr::SProj { n, offset, var } => {
                let obj = self.frame.get_var(*var).clone();
                let size = ir_type_size_from_n(*n);
                let bytes = obj.get_scalar_bytes(*n, *offset, size);
                let mut buf = [0u8; 8];
                let copy_len = size.min(bytes.len()).min(8);
                buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
                Value::Scalar(u64::from_le_bytes(buf))
            }
            Expr::FAp { fun, args } => {
                // Handled by the interpreter's call_function - should not reach here directly
                // This path is for when eval_expr is called directly
                let arg_values: Vec<Value> = args.iter().map(|a| self.resolve_arg(a)).collect();
                let _ = arg_values; // args resolved for trace
                panic!(
                    "FAp should be handled by interpreter.call_function, not eval_expr: {}",
                    fun
                );
            }
            Expr::PAp { fun, args } => {
                let captured: Vec<Value> = args.iter().map(|a| self.resolve_arg(a)).collect();
                Value::Closure {
                    fn_id: fun.clone(),
                    arity: 0, // will be filled from decl lookup
                    captured,
                }
            }
            Expr::Ap { fun, args } => {
                let closure = self.frame.get_var(*fun).clone();
                let extra_args: Vec<Value> = args.iter().map(|a| self.resolve_arg(a)).collect();
                match closure {
                    Value::Closure {
                        fn_id, captured, ..
                    } => {
                        let mut all_args = captured;
                        all_args.extend(extra_args);
                        // Return a marker for the interpreter to handle the call
                        Value::Closure {
                            fn_id,
                            arity: 0,
                            captured: all_args,
                        }
                    }
                    _ => panic!("Ap on non-closure value"),
                }
            }
            Expr::Box { ty, var } => {
                // Box a scalar value into an object with scalar stored in scalars field
                let val = self.frame.get_var(*var).clone();
                let scalar_val = val.as_u64();
                let size = ir_type_size(ty);
                let mut scalars = vec![0u8; size];
                let bytes = scalar_val.to_le_bytes();
                scalars[..size.min(8)].copy_from_slice(&bytes[..size.min(8)]);
                Value::Object {
                    tag: 0,
                    fields: vec![],
                    scalars,
                }
            }
            Expr::Unbox { var } => {
                // Unbox an object to scalar
                let val = self.frame.get_var(*var).clone();
                match &val {
                    Value::Object { scalars, .. } if !scalars.is_empty() => {
                        let mut buf = [0u8; 8];
                        let len = scalars.len().min(8);
                        buf[..len].copy_from_slice(&scalars[..len]);
                        Value::Scalar(u64::from_le_bytes(buf))
                    }
                    Value::Scalar(_) => val, // already a scalar
                    _ => val,
                }
            }
            Expr::Lit { val } => match val {
                LitValue::Num(n) => Value::Scalar(*n),
                LitValue::Str(s) => Value::Str(s.clone()),
            },
            Expr::IsShared { .. } => {
                // RC excluded: always "not shared"
                Value::Scalar(0)
            }
            Expr::Reset { var, .. } => {
                // Identity - treat as pass-through
                self.frame.get_var(*var).clone()
            }
            Expr::Reuse { info, args, .. } => {
                // Treat as Ctor
                self.eval(&Expr::Ctor {
                    info: info.clone(),
                    args: args.clone(),
                })
            }
        }
    }
}

fn ir_type_size_from_n(n: u32) -> usize {
    match n {
        0 => 1, // UInt8
        1 => 2, // UInt16
        2 => 4, // UInt32
        _ => 8, // UInt64 / default
    }
}

fn ir_type_size(ty: &IRType) -> usize {
    match ty {
        IRType::UInt8 => 1,
        IRType::UInt16 => 2,
        IRType::UInt32 | IRType::Float => 4,
        IRType::UInt64 | IRType::USize => 8,
        _ => 8,
    }
}
