---
title: "Proving a Lean STF in a zkVM"
last_updated: 2026-06-03
tags:
  - zkvm
  - lean
  - ethereum
  - stf
  - risc-zero
---

<!-- ethresear.ch post body starts below. Do NOT paste the YAML frontmatter above
     into Discourse — it is repo metadata only and will not render. Copy from the
     H1 heading onward. Replace [link to repo] before posting.

     Figures are SVGs in docs/assets/. On GitHub the relative image links render
     as-is. For ethresear.ch, upload each figure to the post (if your Discourse
     instance rejects SVG uploads, export the SVG to PNG first — e.g. open it in
     a browser and "Save as", or run `rsvg-convert in.svg -o out.png`) and swap
     the image links for the uploaded URLs. -->

# Proving a Lean STF in a zkVM

## 1. TL;DR

Repo: https://github.com/NyxFoundation/verifiable-stf

The Ethereum roadmap explores making the consensus-layer state transition function (STF) provable on a zkVM. If validators and light clients can verify a succinct ZK proof rather than re-executing the STF, node resource requirements drop significantly and the barrier to consensus participation falls.

This logic is central to Ethereum's security model and a natural candidate for formal verification. This post reports on an approach to verifying a formally-specified STF inside a zkVM. We wrote the STF in Lean and explored three ways to get a zk proof of its execution inside a RISC Zero zkVM. The main focus of this post is the third approach: IR Trace.

| Approach | What runs in the guest |
|----------|------------------------|
| Compiled Rust → RISC-V | A hand-written Rust STF (baseline) |
| Compiled Lean → RISC-V | The full STF + Lean runtime + `Init` (~15M cycles of init alone) |
| Lean IR Trace (focus of this post) | A tiny checker that re-verifies a host-generated execution trace |

In the IR Trace approach, even a single validator generates a 7.85 GB trace — far above the ~4 GB input limit. The trace size breaks down as a large fixed cost (~7.81 GB, independent of validator count) plus ~32.7 MB per additional validator. Reducing the validator count does not get under the wall.

We're sharing the numbers and the wall we hit, and asking for feedback on trace-compression directions (and on whether interpret-then-verify is the right shape at all).

Three premises underpin all results reported below — read these before proceeding:

1. The host interpreter is an independent reimplementation of the official C++ IR interpreter. When this post says "we ran the Lean program", it means "we ran the λRC IR through this independent Rust reimplementation" — bit-for-bit equivalence with the official C++ interpreter ([`ir_interpreter.cpp`](https://github.com/leanprover/lean4/blob/master/src/library/ir_interpreter.cpp)) is not established. See "3.1. Host-side interpreter" for the three intentional differences.
2. Crypto is axiomatised as extern stubs. `hashTreeRoot`, `blsVerify`, etc. are accepted at their recorded results on the guest side and not re-executed. These form the trust boundary of this prototype.
3. This implementation proves specific execution instances of the STF in a zkVM; it does not prove properties of the STF as theorems over all possible inputs.

## 2. Motivation

Formal verification proves program correctness mathematically, covering all possible inputs and eliminating bug classes that testing cannot reach. The Ethereum consensus spec is the kind of artifact you'd love to write in a proof assistant: precise semantics, machine-checkable invariants. Lean is attractive for that.

The question we started from: if the STF is written in Lean, how do we get a succinct ZK proof that a given pre-state + block produced a given post-state?

The obvious route is to run the STF inside a zkVM. Lean's compiler can emit C source code, which in principle can be recompiled for RISC-V and run as a zkVM guest program. This approach works but is heavy — the Lean runtime and `Init` alone cost ~15M cycles of initialization in our benchmarks.

## 3. The IR Trace approach

So we asked: can we avoid compiling Lean to RISC-V entirely?

Lean lowers programs to a small IR (lambda-RC, λRC) before emitting C. This IR is sequentially evaluable step-by-step — a chain of `let x := expr; ...` bindings plus `Branch` / `Call` / `Set` primitives, where each step produces one new value from the existing value table. This maps naturally onto the basic zkVM proof flow: re-execute a sequence of steps and obtain a hash-like commitment to the final state. So: interpret the IR on the host, emit a structured execution trace, and shrink the zkVM guest from a full language runtime to a small trace checker. That is the IR Trace approach.

Lean itself already had this idea: it ships an official C++ λRC interpreter ([`ir_interpreter.cpp`](https://github.com/leanprover/lean4/blob/master/src/library/ir_interpreter.cpp)) for platforms where LLVM JIT is unavailable (e.g. WebAssembly). But we could not use it directly: it only executes — it has no mechanism to record per-step intermediate values into a structured trace. So we built an independent Rust reimplementation with trace emit.

### 3.1. Host-side interpreter

The IR Trace pipeline is:

```
Lean STF
   │  dump lambda-RC IR  →  ir_program.json
   ▼
host interpreter  ──  interprets IR on the input, records every step  →  trace (bincode)
   ▼
host driver  ──  hashes IR + input, feeds {ir_hash, input, trace} to the zkVM
   ▼
zkVM guest  ──  re-verifies each trace step, commits the output hash
```

Lean ships an official C++ λRC interpreter, [`src/library/ir_interpreter.cpp`](https://github.com/leanprover/lean4/blob/master/src/library/ir_interpreter.cpp), which is the reference implementation defining the semantics of λRC IR. We could not use it directly for two reasons: (1) it is C++ and does not integrate with the Rust/RISC Zero ecosystem, and (2) it only executes — it has no mechanism to record per-step intermediate values into a structured trace.

We therefore built an independent Rust reimplementation from the same λRC IR spec ([`Lean/Compiler/IR/Basic.lean`](https://github.com/leanprover/lean4/blob/master/src/Lean/Compiler/IR/Basic.lean)) with trace emit wired in. This is not a transliteration of the C++; it is a from-scratch implementation of the same spec. Three intentional differences:

1. Reference-counting ops replaced with clones. `Inc` / `Dec` / `Del` instructions normally manipulate Lean's runtime refcount for memory management. Inside the zkVM there is no GC and no memory reuse, so we read them as `clone()`. The semantics are equivalent; the performance profile is not (clone consumes more memory, but this has no effect on guest-side verification).

2. In-place optimisation ops (`Reset` / `Reuse`) elided. λRC's `Reset` / `Reuse` are compiler-generated optimisations that reclaim memory when the refcount is exactly 1. They have no semantic effect, so we dropped them.

3. `dlsym` native dispatch replaced with extern stubs. The C++ interpreter calls crypto primitives (`blsVerify`, `hashTreeRoot`, etc.) via `dlsym`. Our Rust reimplementation defines these as static extern stubs that return their recorded results — they are not re-executed in the zkVM guest (see "3.2. What the guest actually checks").

As a consequence, when this post says "we ran the Lean program", it means "we ran the λRC IR through this independent Rust reimplementation" — bit-for-bit equivalence with Lean's own C++ interpreter is not established. The coverage gap for in-place mutation ops (`USet` / `SSet` / `SetTag`) is detailed in the next section.

### 3.2. What the guest actually checks

The trace is a flat value table (every intermediate value, referenced by index) plus a list of steps, wrapped in a header carrying SHA-256 hashes of the IR program, the input, and the output. Before trusting anything, the guest asserts those three hashes match, binding the proof to a specific program + input + output.

Trace steps are represented as a 6-variant enum (`ir-trace-common/src/trace_types.rs`):

```rust
pub enum TraceStep {
    Call      { fn_id: u32, args: Vec<ValueId>, result: ValueId },
    Branch    { scrutinee: ValueId, chosen_tag: u16 },
    PrimResult { op: PrimOp, args: Vec<ValueId>, result: ValueId },
    CtorCreate { tag: u16, fields: Vec<ValueId>, scalar_data: Vec<u8>, result: ValueId },
    ProjResult { obj: ValueId, idx: u16, result: ValueId },
    SetResult  { obj: ValueId, idx: u16, val: ValueId, result: ValueId },
}
```

The guest verifier re-checks each step as follows (`methods/guest-ir-trace/src/verifier.rs`, excerpt):

```rust
match step {
    TraceStep::PrimResult { op, args, result } => {
        // recompute the op and compare against the recorded value-table entry
        let computed = eval_primitive(op, &args.iter().map(|id| values[*id as usize].clone()).collect());
        assert_eq!(computed, values[*result as usize]);
    }
    TraceStep::Branch { scrutinee, chosen_tag } => {
        // verify the recorded branch target matches the scrutinee's actual tag
        assert_eq!(values[*scrutinee as usize].tag(), *chosen_tag);
    }
    TraceStep::Call { .. } => {
        // user function: inlined as sub-steps in the trace → checked there
        // extern stubs (hashTreeRoot, blsVerify, etc.): trusted as axioms — not re-executed
    }
    // CtorCreate / ProjResult / SetResult likewise verified with assert_eq!
}
```

- `PrimResult` / `CtorCreate` / `ProjResult` / `SetResult` / `Branch`: fully re-checked against the value table.
- `Call` (user function): the inlined sub-steps are checked, so the call itself passes.
- `Call` (extern stubs — `hashTreeRoot`, `blsVerify`, etc.): recorded results accepted, not re-executed.

Current coverage gap — in-place mutation: be careful not to overstate what is checked. The `Set` field-update op does emit a re-checked `SetResult` step. But the in-place mutation ops `USet` / `SSet` / `SetTag` are executed by the interpreter yet emit no trace step today, so they currently fall outside the verified set. Closing this gap is required before any ETH2 proof would be meaningful. The semantics of `USet` / `SSet` / `SetTag` are implemented in the official C++ interpreter ([`ir_interpreter.cpp`](https://github.com/leanprover/lean4/blob/master/src/library/ir_interpreter.cpp)); porting them to Rust with tracing added is straightforward.

What is being proven, precisely: the proof binds the IR-program hash, the input hash, the output hash, and the checked trace semantics above. It does not establish equivalence to Lean's own runtime, and it does not establish correctness of the stubbed crypto. The trust boundary is exactly the set of extern stubs plus the in-place mutation coverage gap above.

## 4. Benchmark results

Measurement environment: AMD Ryzen 9 PRO 8945HS (8 cores / 16 threads, up to 5.26 GHz), L2 8 MiB / L3 16 MiB, RAM 64 GiB, Linux 6.18 (Kali).

A caveat up front: the three approaches do not share a single cost metric. Host-interpreter statistics (step counts, trace size) exist only for IR Trace; zkVM cycles exist only for the compiled approaches. So we report in two buckets rather than one misleading table.

### 4.1. Trace size (IR Trace)

We measured V=1,2,3 directly and cross-checked against V=10 and V=100:

| V (validators) | Serialized trace | Trace steps | Value-table entries | Output |
|---------------:|-----------------:|------------:|--------------------:|-------:|
| 1   | 7.85 GB              | 229,371 | 617,066 | 77,199 B |
| 2   | 7.88 GB              | 230,334 | 619,593 | 77,346 B |
| 3   | 7.91 GB              | 231,308 | 622,147 | 77,493 B |
| 10  | 8.14 GB              | 238,049 | 639,836 | 78,522 B |
| 100 | ≈11.1 GB (estimated) | 324,741 | 867,320 | 91,752 B |

(Trace bytes are measured at V=1,2,3,10; at V=100 we extrapolate the byte size — serializing a >10 GB trace just to weigh it is not worth it — but the step and value-table counts are measured at all five.)

Scaling structure: a large fixed cost plus a per-validator cost. A linear fit over V=1,2,3 splits every metric cleanly into a validator-independent fixed cost and an additional cost per validator:

```
serialized trace  ≈   7.81 GB  +  32.7 MB  × V
trace steps       ≈   228,400  +     968   × V
value-table       ≈   614,500  +   2,540   × V    (entries)
output size       ≈    77,052  +     147   × V    (bytes)
```

The fit is tight: it predicts 8.14 GB and 238,080 steps at V=10, and it reproduces the V=10 and V=100 output sizes (78,522 B, 91,752 B) exactly. This fixed-plus-per-validator structure holds across the full V=1 to V=100 range.

![Serialized trace size vs validator count: measured points at V=1,2,3,10 lie on a line with a ~7.81 GB intercept at V=0 and a ~32.7 MB/validator slope, extrapolated to ~11 GB at V=100; the whole line sits far above the ~4 GB input limit.](assets/bench-scaling.svg)

This structure has two consequences:

- At the toy scales we can actually run (V ≤ 100), the fixed cost dominates — it is ~96% of the trace at V=10 — and the fixed cost alone exceeds the ~4 GB input limit (7.85 GB even at V=1). Reducing the validator count does not get under the wall.
- At mainnet scale (~1M validators), the per-validator cost also becomes fatal: 32.7 MB/validator extrapolates to tens of TB. Both terms require compression — the fixed cost blocks even a single-validator proof today, and the per-validator cost rules out realistic scale.

(Looking at only V=10→V=100 earlier suggested a mild "1.36×, roughly flat"; that ratio was just the huge fixed cost swamping the per-validator term.)

The input path in this implementation serializes a `Vec<u8>` via `env::write()` with a u32 length prefix, and the host hits a `TryFromIntError` at ~4 GB. The fixed cost of ~7.81 GB alone is already past that limit — even at V=1 the trace is 7.85 GB. This is not a "too many validators" problem we can dodge by testing small; the wall is structural.

![Serialized trace decomposed into a fixed ~7.81 GB structural cost plus a per-validator cost, at V=1, V=10, and V=100; the fixed cost alone sits past the ~4 GB input limit at every validator count.](assets/bench-trace-size-wall.svg)

Why is the fixed cost so large? The interpreter clones every intermediate value into the value table, and ETH2 intermediates include large `ByteArray`s (the beacon state is ~78 KB) and deeply nested `Object`s, many of them near-duplicates (e.g. a byte array modified at a single index becomes a full new copy). The bulk of these clones happen in the validator-count-independent skeleton of the STF, which is why the fixed cost dominates. Switching the trace format from JSON (~15 GB) to bincode only bought ~1.8×, because the cost is dominated by serializing complex nested values, not by JSON syntax overhead. The per-validator cost (~32.7 MB/validator) is the same cloning pathology applied to per-validator state — harmless at V=1 but catastrophic at mainnet's ~1M validators.

Contrast with compiled Lean: there, intermediates live in paged zkVM memory and are never serialized. IR Trace externalizes all computation to the host and must hand every intermediate result to the guest — which is exactly what blows up.

![IR-trace step composition at V=10: PrimResult 42.2%, Call 21.5%, ProjResult 16.7%, Branch 11.4%, CtorCreate 4.6%, SetResult 3.7%, of 238,049 total steps.](assets/bench-step-composition.svg)

`PrimResult` (arithmetic) is the per-validator workhorse — it carries the steepest slope, so its share creeps up with V (41.3% at V=1 → 42.2% at V=10 → 48.5% at V=100). Everything else is dominated by the fixed cost.

### 4.2. Compiled approaches

The compiled approaches produce STF outputs of 78,746 B (V=10) and 91,976 B (V=100), matching IR Trace (see the output column in 4.1) within 224 B. We attribute the difference to the test-input serializer, not a divergence in the STF itself — but it is not yet reconciled.

| Approach | cycles (V=10) | seg (V=10) | cycles (V=100) | seg (V=100) |
|----------|--------------|-----------|---------------|------------|
| Rust (compiled, baseline) | 12,491,509 | 13 | 14,446,747 | 15 |
| Lean (compiled, incl. `Init`) | 26,148,291 | 29 | 35,281,299 | 38 |

![zkVM cycles: compiled Lean vs Rust at V=10 and V=100. Rust is roughly half of Lean's cycle count; IR Trace zkVM verification is not available, blocked by trace size.](assets/bench-zkvm-cycles.svg)

Lean costs roughly 2.1× (V=10) to 2.4× (V=100) more than Rust. Most of the gap is the fixed overhead from the Lean runtime and `Init` (~15M cycles); the STF execution itself is not far from Rust.

## 5. Known limitations

This is a research prototype; some gaps matter for soundness and we want to be upfront:

1. The host interpreter is an independent reimplementation of the official C++ λRC interpreter ([`ir_interpreter.cpp`](https://github.com/leanprover/lean4/blob/master/src/library/ir_interpreter.cpp)), not a faithful port. Bit-for-bit equivalence with Lean's own runtime is not established. See "3.1. Host-side interpreter" for the three intentional differences.
2. Crypto is stubbed and trusted (`hashTreeRoot`, `blsVerify`, …), as noted above.
3. Trace coverage is incomplete for in-place mutations. Be careful here, because it is easy to overstate what is checked: the `Set` field-update op does emit a re-checked `SetResult` step. But the in-place mutation ops `USet` / `SSet` / `SetTag` are executed by the interpreter yet emit no trace step today, so they currently fall outside the verified set. Closing this gap is required before any ETH2 proof would be meaningful.
