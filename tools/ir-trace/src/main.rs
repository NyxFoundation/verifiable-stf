mod interpreter;
mod ir_loader;
mod ir_types;
mod trace_format;

use std::collections::HashMap;
use std::fs;
use std::time::{Duration, Instant};

use clap::Parser;

use ir_trace_common::trace_types::TraceStep;
use ir_trace_common::value::Value;
use interpreter::Interpreter;
use ir_loader::load_ir_program;
use ir_types::Decl;
use trace_format::{build_trace, sha256};

#[derive(Parser)]
#[command(name = "ir-trace", about = "Lean4 lambda-RC IR interpreter with trace generation")]
struct Args {
    #[arg(long, help = "Path to IR program JSON")]
    ir: String,

    #[arg(long, help = "Path to input data file")]
    input: String,

    #[arg(long, help = "Path to output trace file (required unless --bench)")]
    output: Option<String>,

    #[arg(long, default_value = "risc0_main_eth2", help = "Entry point function name")]
    entry: String,

    #[arg(long, help = "Pass input as a u32 scalar instead of ByteArray")]
    scalar_input: Option<u32>,

    #[arg(long, help = "Benchmark mode: skip trace serialization, print timing stats")]
    bench: bool,

    #[arg(long, default_value_t = 1, help = "Number of runs for wall-time measurement")]
    runs: usize,
}

fn main() {
    let args = Args::parse();

    if !args.bench && args.output.is_none() {
        eprintln!("Error: --output is required unless --bench is specified");
        std::process::exit(1);
    }

    // Load IR program (once)
    let ir_json = fs::read_to_string(&args.ir).expect("Failed to read IR program JSON");
    let decls = load_ir_program(&ir_json).expect("Failed to parse IR program");

    // Load input data (once)
    let input_bytes = fs::read(&args.input).expect("Failed to read input file");
    let ir_program_bytes = ir_json.as_bytes();

    eprintln!(
        "Loaded {} IR declarations, input size: {} bytes",
        decls.len(),
        input_bytes.len()
    );
    eprintln!("IR program hash: {:?}", hex::encode(&sha256(ir_program_bytes)));
    eprintln!("Entry point: {}", args.entry);

    if args.bench {
        run_bench(&args, &decls, &input_bytes);
    } else {
        run_trace(&args, decls, &input_bytes, ir_program_bytes);
    }
}

fn make_input_value(scalar_input: Option<u32>, input_bytes: &[u8]) -> Value {
    if let Some(scalar) = scalar_input {
        Value::Scalar(scalar as u64)
    } else {
        Value::ByteArray(input_bytes.to_vec())
    }
}

fn run_bench(
    args: &Args,
    decls: &HashMap<String, Decl>,
    input_bytes: &[u8],
) {
    let mut durations = Vec::with_capacity(args.runs);
    let mut last_result: Option<Value> = None;
    let mut last_value_count = 0usize;
    let mut last_steps: Vec<TraceStep> = Vec::new();

    for run_idx in 0..args.runs {
        let mut interp = Interpreter::new(decls.clone());
        register_runtime_stubs(&mut interp);
        register_eth2_stubs(&mut interp);

        let input_value = make_input_value(args.scalar_input, input_bytes);

        let start = Instant::now();
        let result = interp.call_function(&args.entry, vec![input_value]);
        let elapsed = start.elapsed();

        durations.push(elapsed);
        eprintln!("  Run {}/{}: {:.3}s", run_idx + 1, args.runs, elapsed.as_secs_f64());

        last_value_count = interp.value_registry.table.len();
        if run_idx == 0 {
            last_steps = std::mem::take(&mut interp.trace_steps);
        }
        last_result = Some(result);
    }

    durations.sort();
    let median = durations[durations.len() / 2];
    let min = durations[0];
    let max = durations[durations.len() - 1];

    // Count step categories
    let mut branch_count = 0u64;
    let mut prim_count = 0u64;
    let mut ctor_count = 0u64;
    let mut proj_count = 0u64;
    let mut set_count = 0u64;
    for step in &last_steps {
        match step {
            TraceStep::Branch { .. } => branch_count += 1,
            TraceStep::PrimResult { .. } => prim_count += 1,
            TraceStep::CtorCreate { .. } => ctor_count += 1,
            TraceStep::ProjResult { .. } => proj_count += 1,
            TraceStep::SetResult { .. } => set_count += 1,
        }
    }
    let total_steps = last_steps.len() as u64;

    // Print structured output
    eprintln!();
    eprintln!("=== IR Interpreter Benchmark ===");
    eprintln!("Entry: {}", args.entry);
    eprintln!("Input: {} bytes", input_bytes.len());
    eprintln!();
    eprintln!("=== Timing ({} runs) ===", args.runs);
    eprintln!("  Median: {}", format_duration(median));
    eprintln!("  Min:    {}", format_duration(min));
    eprintln!("  Max:    {}", format_duration(max));
    eprintln!();
    eprintln!("=== Trace Steps ===");
    eprintln!("  Total:      {:>10}", format_count(total_steps));
    eprintln!("  Branch:      {:>10} ({:.1}%)", format_count(branch_count), pct(branch_count, total_steps));
    eprintln!("  PrimResult:  {:>10} ({:.1}%)", format_count(prim_count), pct(prim_count, total_steps));
    eprintln!("  CtorCreate:  {:>10} ({:.1}%)", format_count(ctor_count), pct(ctor_count, total_steps));
    eprintln!("  ProjResult:  {:>10} ({:.1}%)", format_count(proj_count), pct(proj_count, total_steps));
    eprintln!("  SetResult:   {:>10} ({:.1}%)", format_count(set_count), pct(set_count, total_steps));
    eprintln!();
    eprintln!("=== Memory ===");
    eprintln!("  Value table: {} entries", format_count(last_value_count as u64));
    eprintln!();

    // Output summary
    if let Some(result) = &last_result {
        print_output_summary(result);
    }
}

fn run_trace(
    args: &Args,
    decls: HashMap<String, Decl>,
    input_bytes: &[u8],
    ir_program_bytes: &[u8],
) {
    let mut interp = Interpreter::new(decls);
    register_runtime_stubs(&mut interp);
    register_eth2_stubs(&mut interp);

    let input_value = make_input_value(args.scalar_input, input_bytes);
    let result = interp.call_function(&args.entry, vec![input_value]);

    // Build trace
    let output_value_id = interp.value_registry.register(&result);

    let trace = build_trace(
        &mut interp,
        ir_program_bytes,
        input_bytes,
        &result,
        output_value_id,
    );

    // Serialize trace
    let output_path = args.output.as_ref().unwrap();
    let trace_bytes = bincode::serialize(&trace).expect("Failed to serialize trace");
    fs::write(output_path, &trace_bytes).expect("Failed to write trace output");

    eprintln!("Trace written: {} bytes", trace_bytes.len());
    eprintln!("  Steps: {}", trace.steps.len());
    eprintln!("  Values: {}", trace.value_table.len());

    print_output_summary(&result);
}

fn print_output_summary(result: &Value) {
    eprintln!("=== Output ===");
    match result {
        Value::ByteArray(data) => {
            eprintln!("  Size: {} bytes", data.len());
            if !data.is_empty() {
                match data[0] {
                    0xFF => eprintln!("  Status: State decode error"),
                    0xFE => eprintln!("  Status: Block decode error"),
                    0xFD => {
                        let msg = String::from_utf8_lossy(&data[1..]);
                        eprintln!("  Status: STF error: {}", msg);
                    }
                    _ => eprintln!("  Status: Success (first byte: 0x{:02X})", data[0]),
                }
            }
        }
        Value::Scalar(v) => eprintln!("  Output scalar: {}", v),
        _ => eprintln!("  Output: {:?}", result),
    }
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs >= 1.0 {
        format!("{:.2}s", secs)
    } else if secs >= 0.001 {
        format!("{:.2}ms", secs * 1000.0)
    } else {
        format!("{:.0}µs", secs * 1_000_000.0)
    }
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{},{:03},{:03}", n / 1_000_000, (n / 1_000) % 1_000, n % 1_000)
    } else if n >= 1_000 {
        format!("{},{:03}", n / 1_000, n % 1_000)
    } else {
        format!("{}", n)
    }
}

fn pct(part: u64, total: u64) -> f64 {
    if total == 0 { 0.0 } else { (part as f64 / total as f64) * 100.0 }
}

/// Skip leading Irrelevant args (erased type parameters in Lean4 λRC IR).
fn skip_erased_prefix(args: &[Value]) -> &[Value] {
    if matches!(args.first(), Some(Value::Irrelevant)) { &args[1..] } else { args }
}

fn decidable(b: bool) -> Value {
    // Lean Decidable: isFalse = tag 0, isTrue = tag 1
    Value::Object {
        tag: if b { 1 } else { 0 },
        fields: vec![],
        scalars: vec![],
    }
}

fn register_runtime_stubs(interp: &mut Interpreter) {
    // Nat.decLt: Nat -> Nat -> Decidable
    interp.register_extern_stub(
        "Nat.decLt",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() < args[1].as_u64())),
    );

    // Nat.decLe: Nat -> Nat -> Decidable
    interp.register_extern_stub(
        "Nat.decLe",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() <= args[1].as_u64())),
    );

    // Nat.decEq: Nat -> Nat -> Decidable
    interp.register_extern_stub(
        "Nat.decEq",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() == args[1].as_u64())),
    );

    // ByteArray.extract: ByteArray -> Nat -> Nat -> ByteArray
    interp.register_extern_stub(
        "ByteArray.extract",
        Box::new(|args: &[Value]| {
            let ba = match &args[0] {
                Value::ByteArray(v) => v,
                _ => return Value::ByteArray(vec![]),
            };
            let start = args[1].as_u64() as usize;
            let stop = args[2].as_u64() as usize;
            let start = start.min(ba.len());
            let stop = stop.min(ba.len());
            if start >= stop {
                Value::ByteArray(vec![])
            } else {
                Value::ByteArray(ba[start..stop].to_vec())
            }
        }),
    );

    // String.decEq: String -> String -> Decidable
    interp.register_extern_stub(
        "String.decEq",
        Box::new(|args: &[Value]| {
            match (&args[0], &args[1]) {
                (Value::Str(a), Value::Str(b)) => decidable(a == b),
                _ => decidable(false),
            }
        }),
    );

    // USize.decEq, USize.decLt
    interp.register_extern_stub(
        "USize.decEq",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() == args[1].as_u64())),
    );
    interp.register_extern_stub(
        "USize.decLt",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() < args[1].as_u64())),
    );

    // Array.mkArray: {α} → Nat → α → Array α
    interp.register_extern_stub(
        "Array.mkArray",
        Box::new(|args: &[Value]| {
            let a = skip_erased_prefix(args);
            assert!(a.len() >= 2, "Array.mkArray: expected >=2 args after skip, got {}", a.len());
            let n = a[0].as_u64() as usize;
            let val = a[1].clone();
            Value::Array(vec![val; n])
        }),
    );

    // ByteArray.mk: List UInt8 -> ByteArray
    interp.register_extern_stub(
        "ByteArray.mk",
        Box::new(|args: &[Value]| {
            // List is a linked list: nil=tag0, cons=tag1(head,tail)
            let mut bytes = Vec::new();
            let mut current = &args[0];
            loop {
                match current {
                    Value::Object { tag: 0, .. } => break,
                    Value::Object {
                        tag: 1, fields, ..
                    } => {
                        if let Some(head) = fields.first() {
                            bytes.push(head.as_u64() as u8);
                        }
                        if let Some(tail) = fields.get(1) {
                            current = tail;
                        } else {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            Value::ByteArray(bytes)
        }),
    );

    // String.toUTF8: String -> ByteArray
    interp.register_extern_stub(
        "String.toUTF8",
        Box::new(|args: &[Value]| match &args[0] {
            Value::Str(s) => Value::ByteArray(s.as_bytes().to_vec()),
            _ => Value::ByteArray(vec![]),
        }),
    );

    // String.data: String -> List Char (simplified)
    interp.register_extern_stub(
        "String.data",
        Box::new(|_args: &[Value]| {
            // Return empty list
            Value::Object {
                tag: 0,
                fields: vec![],
                scalars: vec![],
            }
        }),
    );

    // UInt32.decEq / UInt64.decEq
    interp.register_extern_stub(
        "UInt32.decEq",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() as u32 == args[1].as_u64() as u32)),
    );
    interp.register_extern_stub(
        "UInt64.decEq",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() == args[1].as_u64())),
    );

    // Nat.ble / Nat.blt / Nat.beq (return Bool as object tag 0/1)
    interp.register_extern_stub(
        "Nat.ble",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() <= args[1].as_u64())),
    );
    interp.register_extern_stub(
        "Nat.blt",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() < args[1].as_u64())),
    );

    // ByteArray.empty / Array.empty
    interp.register_extern_stub(
        "ByteArray.empty",
        Box::new(|_args: &[Value]| Value::ByteArray(vec![])),
    );
    interp.register_extern_stub(
        "Array.empty",
        Box::new(|_args: &[Value]| Value::Array(vec![])),
    );

    // UInt64.ofNatLT: proof-carrying, just return the nat as u64
    interp.register_extern_stub(
        "UInt64.ofNatLT",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64())),
    );

    // UInt32.ofNatLT
    interp.register_extern_stub(
        "UInt32.ofNatLT",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64() & 0xFFFFFFFF)),
    );

    // UInt8.ofNatLT
    interp.register_extern_stub(
        "UInt8.ofNatLT",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64() & 0xFF)),
    );

    // List.lengthTRAux: {α} → List α → Nat → Nat
    interp.register_extern_stub(
        "List.lengthTRAux",
        Box::new(|args: &[Value]| {
            let a = skip_erased_prefix(args);
            assert!(a.len() >= 2, "List.lengthTRAux: expected >=2 args after skip, got {}", a.len());
            let mut count = a[1].as_u64();
            let mut current = &a[0];
            loop {
                match current {
                    Value::Object { tag: 0, .. } => break,
                    Value::Object { tag: 1, fields, .. } => {
                        count += 1;
                        if let Some(tail) = fields.get(1) {
                            current = tail;
                        } else {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            Value::Scalar(count)
        }),
    );

    // Fin.val: extract the nat from a Fin
    interp.register_extern_stub(
        "Fin.val",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64())),
    );

    // System.Platform.numBits: return 64
    interp.register_extern_stub(
        "System.Platform.numBits",
        Box::new(|_args: &[Value]| Value::Scalar(64)),
    );

    // USize.ofNat
    interp.register_extern_stub(
        "USize.ofNat",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64())),
    );

    // USize.toNat
    interp.register_extern_stub(
        "USize.toNat",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64())),
    );

    // USize.add/sub/mul/mod/div
    interp.register_extern_stub(
        "USize.add",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64().wrapping_add(args[1].as_u64()))),
    );
    interp.register_extern_stub(
        "USize.sub",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64().wrapping_sub(args[1].as_u64()))),
    );
    interp.register_extern_stub(
        "USize.mul",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64().wrapping_mul(args[1].as_u64()))),
    );
    interp.register_extern_stub(
        "USize.mod",
        Box::new(|args: &[Value]| {
            let b = args[1].as_u64();
            Value::Scalar(if b == 0 { 0 } else { args[0].as_u64() % b })
        }),
    );
    interp.register_extern_stub(
        "USize.div",
        Box::new(|args: &[Value]| {
            let b = args[1].as_u64();
            Value::Scalar(if b == 0 { 0 } else { args[0].as_u64() / b })
        }),
    );
    interp.register_extern_stub(
        "USize.land",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64() & args[1].as_u64())),
    );
    interp.register_extern_stub(
        "USize.lor",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64() | args[1].as_u64())),
    );
    interp.register_extern_stub(
        "USize.shiftLeft",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64().wrapping_shl(args[1].as_u64() as u32))),
    );
    interp.register_extern_stub(
        "USize.shiftRight",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64().wrapping_shr(args[1].as_u64() as u32))),
    );
    interp.register_extern_stub(
        "USize.beq",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() == args[1].as_u64())),
    );
    interp.register_extern_stub(
        "USize.blt",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() < args[1].as_u64())),
    );
    interp.register_extern_stub(
        "USize.ble",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() <= args[1].as_u64())),
    );

    // Char.ofNat
    interp.register_extern_stub(
        "Char.ofNat",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64())),
    );

    // UInt64.decLe / UInt64.decLt
    interp.register_extern_stub(
        "UInt64.decLe",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() <= args[1].as_u64())),
    );
    interp.register_extern_stub(
        "UInt64.decLt",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() < args[1].as_u64())),
    );

    // UInt32.decLe / UInt32.decLt
    interp.register_extern_stub(
        "UInt32.decLe",
        Box::new(|args: &[Value]| decidable((args[0].as_u64() as u32) <= (args[1].as_u64() as u32))),
    );
    interp.register_extern_stub(
        "UInt32.decLt",
        Box::new(|args: &[Value]| decidable((args[0].as_u64() as u32) < (args[1].as_u64() as u32))),
    );

    // ByteArray.uget: ByteArray -> USize -> UInt8
    interp.register_extern_stub(
        "ByteArray.uget",
        Box::new(|args: &[Value]| {
            let ba = match &args[0] {
                Value::ByteArray(v) => v,
                _ => return Value::Scalar(0),
            };
            let idx = args[1].as_u64() as usize;
            Value::Scalar(*ba.get(idx).unwrap_or(&0) as u64)
        }),
    );

    // ByteArray.uset: ByteArray -> USize -> UInt8 -> ByteArray
    interp.register_extern_stub(
        "ByteArray.uset",
        Box::new(|args: &[Value]| {
            let mut ba = match &args[0] {
                Value::ByteArray(v) => v.clone(),
                _ => return Value::ByteArray(vec![]),
            };
            let idx = args[1].as_u64() as usize;
            let val = args[2].as_u64() as u8;
            if idx < ba.len() {
                ba[idx] = val;
            }
            Value::ByteArray(ba)
        }),
    );

    // Array.uget: {α} → Array α → USize → α  (IR: [_, arr, idx, _proof])
    interp.register_extern_stub(
        "Array.uget",
        Box::new(|args: &[Value]| {
            let a = skip_erased_prefix(args);
            assert!(a.len() >= 2, "Array.uget: expected >=2 args after skip, got {}", a.len());
            match &a[0] {
                Value::Array(v) => {
                    let idx = a[1].as_u64() as usize;
                    v.get(idx).cloned().unwrap_or(Value::Irrelevant)
                }
                other => panic!("Array.uget: expected Array, got {:?}", other),
            }
        }),
    );

    // Array.uset: {α} → Array α → USize → α → Array α
    interp.register_extern_stub(
        "Array.uset",
        Box::new(|args: &[Value]| {
            let a = skip_erased_prefix(args);
            assert!(a.len() >= 3, "Array.uset: expected >=3 args after skip, got {}", a.len());
            match &a[0] {
                Value::Array(v) => {
                    let idx = a[1].as_u64() as usize;
                    let val = a[2].clone();
                    let mut new_arr = v.clone();
                    if idx < new_arr.len() {
                        new_arr[idx] = val;
                    }
                    Value::Array(new_arr)
                }
                other => panic!("Array.uset: expected Array, got {:?}", other),
            }
        }),
    );

    // Array.fget: {α} → Array α → Fin n → α
    interp.register_extern_stub(
        "Array.fget",
        Box::new(|args: &[Value]| {
            let a = skip_erased_prefix(args);
            assert!(a.len() >= 2, "Array.fget: expected >=2 args after skip, got {}", a.len());
            match &a[0] {
                Value::Array(v) => {
                    let idx = a[1].as_u64() as usize;
                    v.get(idx).cloned().unwrap_or(Value::Irrelevant)
                }
                other => panic!("Array.fget: expected Array, got {:?}", other),
            }
        }),
    );

    // Array.fset: {α} → Array α → Fin n → α → Array α
    interp.register_extern_stub(
        "Array.fset",
        Box::new(|args: &[Value]| {
            let a = skip_erased_prefix(args);
            assert!(a.len() >= 3, "Array.fset: expected >=3 args after skip, got {}", a.len());
            match &a[0] {
                Value::Array(v) => {
                    let idx = a[1].as_u64() as usize;
                    let val = a[2].clone();
                    let mut new_arr = v.clone();
                    if idx < new_arr.len() {
                        new_arr[idx] = val;
                    }
                    Value::Array(new_arr)
                }
                other => panic!("Array.fset: expected Array, got {:?}", other),
            }
        }),
    );

    // UInt8.decEq / UInt8.decLt / UInt8.decLe
    interp.register_extern_stub(
        "UInt8.decEq",
        Box::new(|args: &[Value]| decidable((args[0].as_u64() as u8) == (args[1].as_u64() as u8))),
    );
    interp.register_extern_stub(
        "UInt8.decLt",
        Box::new(|args: &[Value]| decidable((args[0].as_u64() as u8) < (args[1].as_u64() as u8))),
    );

    // UInt16.decEq
    interp.register_extern_stub(
        "UInt16.decEq",
        Box::new(|args: &[Value]| decidable((args[0].as_u64() as u16) == (args[1].as_u64() as u16))),
    );

    // Nat.add/sub/mul etc. as externs (boxed variants)
    interp.register_extern_stub(
        "Nat.add",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64().wrapping_add(args[1].as_u64()))),
    );
    interp.register_extern_stub(
        "Nat.sub",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64().saturating_sub(args[1].as_u64()))),
    );
    interp.register_extern_stub(
        "Nat.mul",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64().wrapping_mul(args[1].as_u64()))),
    );
    interp.register_extern_stub(
        "Nat.div",
        Box::new(|args: &[Value]| {
            let b = args[1].as_u64();
            Value::Scalar(if b == 0 { 0 } else { args[0].as_u64() / b })
        }),
    );
    interp.register_extern_stub(
        "Nat.mod",
        Box::new(|args: &[Value]| {
            let b = args[1].as_u64();
            Value::Scalar(if b == 0 { 0 } else { args[0].as_u64() % b })
        }),
    );
    interp.register_extern_stub(
        "Nat.beq",
        Box::new(|args: &[Value]| decidable(args[0].as_u64() == args[1].as_u64())),
    );
    interp.register_extern_stub(
        "Nat.land",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64() & args[1].as_u64())),
    );
    interp.register_extern_stub(
        "Nat.lor",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64() | args[1].as_u64())),
    );
    interp.register_extern_stub(
        "Nat.xor",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64() ^ args[1].as_u64())),
    );
    interp.register_extern_stub(
        "Nat.shiftLeft",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64().wrapping_shl(args[1].as_u64() as u32))),
    );
    interp.register_extern_stub(
        "Nat.shiftRight",
        Box::new(|args: &[Value]| Value::Scalar(args[0].as_u64().wrapping_shr(args[1].as_u64() as u32))),
    );

    // Nat.pow
    interp.register_extern_stub(
        "Nat.pow",
        Box::new(|args: &[Value]| {
            let base = args[0].as_u64();
            let exp = args[1].as_u64() as u32;
            Value::Scalar(base.wrapping_pow(exp))
        }),
    );

    // Nat.log2
    interp.register_extern_stub(
        "Nat.log2",
        Box::new(|args: &[Value]| {
            let n = args[0].as_u64();
            Value::Scalar(if n <= 1 { 0 } else { 63 - n.leading_zeros() as u64 })
        }),
    );

    // Array.get!Internal: {α} → Array α → Nat → α → α
    // IR may pass: [_, _, arr, idx, default] or [_, arr, idx, default] or [arr, idx, default]
    interp.register_extern_stub(
        "Array.get!Internal",
        Box::new(|args: &[Value]| {
            // Find the Array argument (skip leading Irrelevant/non-Array args)
            let arr_idx = args.iter().position(|a| matches!(a, Value::Array(_)));
            if let Some(ai) = arr_idx {
                match &args[ai] {
                    Value::Array(v) => {
                        let idx = if ai + 1 < args.len() { args[ai + 1].as_u64() as usize } else { 0 };
                        let default = if ai + 2 < args.len() { &args[ai + 2] } else { &Value::Irrelevant };
                        if idx < v.len() {
                            v[idx].clone()
                        } else {
                            default.clone()
                        }
                    }
                    _ => unreachable!(),
                }
            } else {
                // No Array found — return last arg as default
                args.last().cloned().unwrap_or(Value::Irrelevant)
            }
        }),
    );

    // Array.replicate: {α} → Nat → α → Array α
    interp.register_extern_stub(
        "Array.replicate",
        Box::new(|args: &[Value]| {
            let a = skip_erased_prefix(args);
            assert!(a.len() >= 2, "Array.replicate: expected >=2 args after skip, got {}", a.len());
            let n = a[0].as_u64() as usize;
            let val = a[1].clone();
            Value::Array(vec![val; n])
        }),
    );

    // ByteArray equality (mangled name from Lean compiler)
    interp.register_extern_stub(
        "ByteArray.beqByteArray._@.Init.Data.ByteArray.Basic._hyg.25",
        Box::new(|args: &[Value]| {
            match (&args[0], &args[1]) {
                (Value::ByteArray(a), Value::ByteArray(b)) => decidable(a == b),
                _ => decidable(false),
            }
        }),
    );

    // instInhabitedUInt64: default UInt64 value (0)
    interp.register_extern_stub(
        "instInhabitedUInt64",
        Box::new(|_args: &[Value]| Value::Scalar(0)),
    );

    // Array.usize: {α} → Array α → USize (returns array length)
    interp.register_extern_stub(
        "Array.usize",
        Box::new(|args: &[Value]| {
            let a = skip_erased_prefix(args);
            match &a[0] {
                Value::Array(v) => Value::Scalar(v.len() as u64),
                _ => Value::Scalar(0),
            }
        }),
    );
}

fn register_eth2_stubs(interp: &mut Interpreter) {
    // hashTreeRoot: ByteArray -> ByteArray (returns 32-byte hash)
    interp.register_extern_stub(
        "Eth2.Crypto.hashTreeRoot",
        Box::new(|args: &[Value]| {
            // Stub: SHA-256 of the input bytes
            let input = match &args[0] {
                Value::ByteArray(data) => data.clone(),
                _ => vec![],
            };
            let hash = trace_format::sha256(&input);
            Value::ByteArray(hash.to_vec())
        }),
    );

    // BLS verify stub (always returns true in trace mode)
    interp.register_extern_stub(
        "Eth2.Crypto.blsVerify",
        Box::new(|_args: &[Value]| {
            Value::Object {
                tag: 1, // true
                fields: vec![],
                scalars: vec![],
            }
        }),
    );

    // hash256 stub
    interp.register_extern_stub(
        "Eth2.Crypto.hash256",
        Box::new(|args: &[Value]| {
            let input = match &args[0] {
                Value::ByteArray(data) => data.clone(),
                _ => vec![],
            };
            let hash = trace_format::sha256(&input);
            Value::ByteArray(hash.to_vec())
        }),
    );
}

mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arr(vals: Vec<u64>) -> Value {
        Value::Array(vals.into_iter().map(Value::Scalar).collect())
    }

    fn list(vals: Vec<u64>) -> Value {
        let mut result = Value::Object { tag: 0, fields: vec![], scalars: vec![] };
        for v in vals.into_iter().rev() {
            result = Value::Object { tag: 1, fields: vec![Value::Scalar(v), result], scalars: vec![] };
        }
        result
    }

    #[test]
    fn test_stub_array_uget() {
        let stub = |args: &[Value]| -> Value {
            let a = skip_erased_prefix(args);
            match &a[0] {
                Value::Array(v) => {
                    let idx = a[1].as_u64() as usize;
                    v.get(idx).cloned().unwrap_or(Value::Irrelevant)
                }
                other => panic!("expected Array, got {:?}", other),
            }
        };
        // With erased prefix
        let result = stub(&[Value::Irrelevant, arr(vec![10, 20, 30]), Value::Scalar(1)]);
        assert_eq!(result.as_u64(), 20);
        // Without erased prefix
        let result = stub(&[arr(vec![10, 20, 30]), Value::Scalar(2)]);
        assert_eq!(result.as_u64(), 30);
    }

    #[test]
    fn test_stub_array_uset() {
        let stub = |args: &[Value]| -> Value {
            let a = skip_erased_prefix(args);
            match &a[0] {
                Value::Array(v) => {
                    let idx = a[1].as_u64() as usize;
                    let val = a[2].clone();
                    let mut new_arr = v.clone();
                    if idx < new_arr.len() { new_arr[idx] = val; }
                    Value::Array(new_arr)
                }
                other => panic!("expected Array, got {:?}", other),
            }
        };
        let result = stub(&[Value::Irrelevant, arr(vec![10, 20, 30]), Value::Scalar(1), Value::Scalar(99)]);
        match result {
            Value::Array(v) => assert_eq!(v[1].as_u64(), 99),
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn test_stub_array_fget() {
        let stub = |args: &[Value]| -> Value {
            let a = skip_erased_prefix(args);
            match &a[0] {
                Value::Array(v) => {
                    let idx = a[1].as_u64() as usize;
                    v.get(idx).cloned().unwrap_or(Value::Irrelevant)
                }
                other => panic!("expected Array, got {:?}", other),
            }
        };
        // Fin is passed as Scalar(idx) in IR
        let result = stub(&[Value::Irrelevant, arr(vec![100, 200, 300]), Value::Scalar(2)]);
        assert_eq!(result.as_u64(), 300);
    }

    #[test]
    fn test_stub_array_fset() {
        let stub = |args: &[Value]| -> Value {
            let a = skip_erased_prefix(args);
            match &a[0] {
                Value::Array(v) => {
                    let idx = a[1].as_u64() as usize;
                    let val = a[2].clone();
                    let mut new_arr = v.clone();
                    if idx < new_arr.len() { new_arr[idx] = val; }
                    Value::Array(new_arr)
                }
                other => panic!("expected Array, got {:?}", other),
            }
        };
        let result = stub(&[Value::Irrelevant, arr(vec![1, 2, 3]), Value::Scalar(0), Value::Scalar(42)]);
        match result {
            Value::Array(v) => assert_eq!(v[0].as_u64(), 42),
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn test_stub_array_mkarray() {
        let stub = |args: &[Value]| -> Value {
            let a = skip_erased_prefix(args);
            let n = a[0].as_u64() as usize;
            let val = a[1].clone();
            Value::Array(vec![val; n])
        };
        let result = stub(&[Value::Irrelevant, Value::Scalar(3), Value::Scalar(7)]);
        match result {
            Value::Array(v) => {
                assert_eq!(v.len(), 3);
                assert!(v.iter().all(|x| x.as_u64() == 7));
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn test_stub_list_length_tr_aux_zero_acc() {
        let stub = |args: &[Value]| -> Value {
            let a = skip_erased_prefix(args);
            let mut count = a[1].as_u64();
            let mut current = &a[0];
            loop {
                match current {
                    Value::Object { tag: 0, .. } => break,
                    Value::Object { tag: 1, fields, .. } => {
                        count += 1;
                        if let Some(tail) = fields.get(1) { current = tail; } else { break; }
                    }
                    _ => break,
                }
            }
            Value::Scalar(count)
        };
        let result = stub(&[Value::Irrelevant, list(vec![1, 2, 3]), Value::Scalar(0)]);
        assert_eq!(result.as_u64(), 3);
    }

    #[test]
    fn test_stub_list_length_tr_aux_nonzero_acc() {
        let stub = |args: &[Value]| -> Value {
            let a = skip_erased_prefix(args);
            let mut count = a[1].as_u64();
            let mut current = &a[0];
            loop {
                match current {
                    Value::Object { tag: 0, .. } => break,
                    Value::Object { tag: 1, fields, .. } => {
                        count += 1;
                        if let Some(tail) = fields.get(1) { current = tail; } else { break; }
                    }
                    _ => break,
                }
            }
            Value::Scalar(count)
        };
        let result = stub(&[Value::Irrelevant, list(vec![1, 2]), Value::Scalar(5)]);
        assert_eq!(result.as_u64(), 7);
    }
}
