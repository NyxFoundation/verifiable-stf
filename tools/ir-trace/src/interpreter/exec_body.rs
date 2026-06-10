use ir_trace_common::trace_types::TraceStep;

use crate::ir_types::{Alt, Arg, FnBody, IRType};

use super::eval_expr::ExprEvaluator;
use super::stack::CallFrame;
use super::value::Value;
use super::ValueRegistry;

pub struct BodyExecutor<'a> {
    pub frame: &'a mut CallFrame,
    pub trace_steps: &'a mut Vec<TraceStep>,
    pub value_registry: &'a mut ValueRegistry,
}

impl<'a> BodyExecutor<'a> {
    fn resolve_arg(&self, arg: &Arg) -> Value {
        match arg {
            Arg::Var(v) => self.frame.get_var(*v).clone(),
            Arg::Irrelevant => Value::Irrelevant,
        }
    }

    fn eval_expr_inline_with_ty(
        &mut self,
        expr: &crate::ir_types::Expr,
        ty: &IRType,
    ) -> Value {
        let mut evaluator = ExprEvaluator {
            frame: self.frame,
            trace_steps: self.trace_steps,
            value_registry: self.value_registry,
        };
        evaluator.eval_with_ty(expr, ty)
    }

    pub fn exec(&mut self, body: &FnBody) -> ExecResult {
        match body {
            FnBody::VDecl {
                var,
                ty,
                expr,
                cont,
                ..
            } => {
                // Check if this is a FAp or Ap that needs function call handling
                match expr {
                    crate::ir_types::Expr::FAp { fun, args } => {
                        let arg_values: Vec<Value> =
                            args.iter().map(|a| self.resolve_arg(a)).collect();
                        return ExecResult::Call {
                            target: fun.clone(),
                            args: arg_values,
                            result_var: *var,
                            cont: *cont.clone(),
                        };
                    }
                    crate::ir_types::Expr::Ap { fun, args } => {
                        let closure = self.frame.get_var(*fun).clone();
                        let extra_args: Vec<Value> =
                            args.iter().map(|a| self.resolve_arg(a)).collect();
                        match closure {
                            Value::Closure {
                                fn_id, captured, ..
                            } => {
                                let mut all_args = captured;
                                all_args.extend(extra_args);
                                return ExecResult::Call {
                                    target: fn_id,
                                    args: all_args,
                                    result_var: *var,
                                    cont: *cont.clone(),
                                };
                            }
                            _ => panic!("Ap on non-closure value"),
                        }
                    }
                    _ => {}
                }

                // Record projection provenance for reference propagation
                if let crate::ir_types::Expr::Proj { idx, var: src_var } = expr {
                    self.frame.record_proj(*var, *src_var, *idx as usize);
                }

                let val = self.eval_expr_inline_with_ty(expr, ty);
                self.frame.set_var(*var, val);
                self.exec(cont)
            }
            FnBody::JDecl {
                jp,
                params,
                body: jp_body,
                cont,
            } => {
                self.frame
                    .register_jp(*jp, params.clone(), *jp_body.clone());
                self.exec(cont)
            }
            FnBody::Set {
                var,
                idx,
                val,
                cont,
            } => {
                let mut obj = self.frame.get_var(*var).clone();
                let new_val = self.resolve_arg(val);
                let obj_id = self.value_registry.register(&obj);
                let val_id = self.value_registry.register(&new_val);
                obj.set_field(*idx as usize, new_val);
                let result_id = self.value_registry.register(&obj);
                self.trace_steps.push(TraceStep::SetResult {
                    obj: obj_id,
                    idx: *idx as u16,
                    val: val_id,
                    result: result_id,
                });
                self.frame.set_var(*var, obj);
                self.frame.propagate_set(*var);
                self.exec(cont)
            }
            FnBody::USet {
                var,
                idx,
                val,
                cont,
            } => {
                let mut obj = self.frame.get_var(*var).clone();
                let scalar_val = self.frame.get_var(*val).as_u64();
                let offset = *idx as usize * 8;
                obj.set_scalar_bytes(0, offset as u32, &scalar_val.to_le_bytes());
                self.frame.set_var(*var, obj);
                self.frame.propagate_set(*var);
                self.exec(cont)
            }
            FnBody::SSet {
                var,
                n: _,
                offset,
                val,
                ty,
                cont,
            } => {
                let mut obj = self.frame.get_var(*var).clone();
                let scalar_val = self.frame.get_var(*val).as_u64();
                let size = ir_type_byte_size(ty);
                let bytes = &scalar_val.to_le_bytes()[..size];
                obj.set_scalar_bytes(0, *offset, bytes);
                self.frame.set_var(*var, obj);
                self.frame.propagate_set(*var);
                self.exec(cont)
            }
            FnBody::SetTag { var, cidx, cont } => {
                let mut obj = self.frame.get_var(*var).clone();
                match &mut obj {
                    Value::Object { tag, .. } => *tag = *cidx,
                    _ => {}
                }
                self.frame.set_var(*var, obj);
                self.frame.propagate_set(*var);
                self.exec(cont)
            }
            FnBody::Case {
                scrutinee, alts, ..
            } => {
                let val = self.frame.get_var(*scrutinee).clone();
                let tag = val.tag();
                let scrutinee_id = self.value_registry.register(&val);
                self.trace_steps.push(TraceStep::Branch {
                    scrutinee: scrutinee_id,
                    chosen_tag: tag,
                });
                let alt = find_matching_alt(alts, tag);
                self.exec(alt.body())
            }
            FnBody::Ret { arg } => {
                let val = self.resolve_arg(arg);
                ExecResult::Return(val)
            }
            FnBody::Jmp { jp, args } => {
                let jp_def = self.frame.get_jp(*jp).clone();
                let arg_values: Vec<Value> = args.iter().map(|a| self.resolve_arg(a)).collect();
                for (param, val) in jp_def.params.iter().zip(arg_values) {
                    self.frame.set_var(param.var, val);
                }
                self.exec(&jp_def.body)
            }
            FnBody::Unreachable => {
                panic!("Reached unreachable code");
            }
        }
    }
}

pub enum ExecResult {
    Return(Value),
    Call {
        target: String,
        args: Vec<Value>,
        result_var: u32,
        cont: FnBody,
    },
}

fn find_matching_alt(alts: &[Alt], tag: u16) -> &Alt {
    for alt in alts {
        match alt {
            Alt::Ctor { info, .. } if info.cidx == tag => return alt,
            Alt::Default { .. } => return alt,
            _ => {}
        }
    }
    alts.last().expect("No matching alt found")
}

fn ir_type_byte_size(ty: &IRType) -> usize {
    match ty {
        IRType::UInt8 => 1,
        IRType::UInt16 => 2,
        IRType::UInt32 | IRType::Float => 4,
        IRType::UInt64 | IRType::USize => 8,
        _ => 8,
    }
}
