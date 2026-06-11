#![no_main]
risc0_zkvm::guest::entry!(main);

use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};

use ir_trace_common::trace_types::Trace;

mod executor;

fn main() {
    // Read inputs from host
    let ir_program_hash: [u8; 32] = env::read();
    let input: Vec<u8> = env::read();
    let trace_bytes: Vec<u8> = env::read();

    // Deserialize trace (steps + seed values only; the full value table is
    // reconstructed by re-executing the steps below).
    let trace: Trace =
        bincode::deserialize(&trace_bytes).expect("Failed to deserialize trace");

    // 1. Bind the proof to the IR program.
    assert_eq!(
        trace.header.ir_program_hash, ir_program_hash,
        "IR program hash mismatch"
    );

    // 2. Bind the proof to the input.
    let input_hash = sha256(&input);
    assert_eq!(
        trace.header.input_hash, input_hash,
        "Input hash mismatch"
    );

    // 3. Reconstruct the value table by re-executing the trace.
    let values = executor::execute_trace(&trace);

    // 4. Commit the output, binding it to the recorded output hash.
    let output_value = &values[trace.output_value_id as usize];
    let output_bytes = output_value.serialize_to_bytes();
    let output_hash = sha256(&output_bytes);
    assert_eq!(
        trace.header.output_hash, output_hash,
        "Output hash mismatch"
    );

    env::commit_slice(&output_bytes);
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}
