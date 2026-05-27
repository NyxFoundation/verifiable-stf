---
title: "Verifying a Lean-specified Ethereum STF in a zkVM by interpreting IR and proving the trace"
last_updated: 2026-05-27
tags:
  - zkvm
  - lean
  - ethereum
  - stf
  - risc-zero
---

<!-- ethresear.ch post body starts below. Do NOT paste the YAML frontmatter above
     into Discourse — it is repo metadata only and will not render. Copy from the
     H1 heading onward. Replace [link to repo] before posting. -->

# Verifying a Lean-specified Ethereum STF in a zkVM by interpreting IR and proving the trace

## TL;DR

- We wrote an Ethereum consensus **state transition function (STF) in Lean 4** and explored three ways to get a zk proof of its execution inside a RISC Zero zkVM: compile Lean→RISC-V, compile Rust→RISC-V (baseline), and a third **"IR Trace"** approach that needs no Lean→RISC-V toolchain.
- In the **IR Trace** approach the host *interprets* Lean's lambda-RC (λRC) IR and records an execution trace; the zkVM guest is a tiny program that **re-checks each trace step**. A toy `sum` program proves end-to-end at **653,173 cycles**.
- For the real ETH2 STF, **host-side trace generation completes**, but the trace is **8.14 GB** (serialized bincode) — over the input-path limit we hit in this implementation — so the ETH2 STF has **not** yet been executed or proven in-guest.
- We're sharing the numbers and the wall we hit, and **asking for feedback** on trace-compression directions (and on whether interpret-then-verify is the right shape at all).

> Note on terminology: below, "the host generated a trace" means trace generation ran to completion on the host. It does **not** imply the trace was verified — for ETH2 the in-guest verifier has not run.

## Motivation

The Ethereum consensus spec is the kind of artifact you'd love to write in a proof assistant: precise semantics, machine-checkable invariants. Lean 4 is attractive for that. The question we started from: **if the STF is written in Lean, how do we get a succinct proof that a given pre-state + block produced a given post-state?**

The obvious route is to run the STF inside a zkVM. But getting Lean *into* a RISC-V zkVM guest is heavy: you either compile Lean→C→RISC-V (and drag in the Lean runtime + `Init`), or you re-implement the STF in a zkVM-friendly language and lose the link to the Lean spec.

So we asked: **can we avoid compiling Lean to RISC-V entirely?** Lean already lowers programs to a small IR (lambda-RC). What if we interpret that IR *on the host*, emit a structured execution trace, and make the zkVM guest a small **trace checker** instead of a full language runtime? That is the IR Trace approach.

## The three approaches

| Approach | What runs in the guest |
|----------|------------------------|
| **Compiled Lean → RISC-V** | The full STF + Lean runtime + `Init` (~15M cycles of init alone) |
| **Compiled Rust → RISC-V** | A hand-written Rust STF (baseline) |
| **IR Trace** (focus of this post) | A tiny checker that re-verifies a host-generated execution trace |

The IR Trace pipeline is one-directional:

```
Lean 4 STF
   │  dump lambda-RC IR  →  ir_program.json
   ▼
host interpreter  ──  interprets IR on the input, records every step  →  trace (bincode)
   ▼
host driver  ──  hashes IR + input, feeds {ir_hash, input, trace} to the zkVM
   ▼
zkVM guest  ──  re-verifies each trace step, commits the output hash
```

## What the guest actually checks

The trace is a flat **value table** (every intermediate value, referenced by index) plus a list of **steps**, wrapped in a header carrying SHA-256 hashes of the IR program, the input, and the output. Before trusting anything, the guest asserts those three hashes match, binding the proof to a specific program + input + output.

The verifier re-checks trace steps as follows:

- **Fully re-checked against the value table:** `PrimResult` (primitive ops), `CtorCreate` (constructor cells), `ProjResult` (field projection), `SetResult` (the `Set` field-update op), and `Branch` (the chosen constructor tag matches the scrutinee).
- **Function calls are *not* trusted as black boxes:** a user-function `Call` is represented by its inlined sub-steps in the trace, which are themselves checked. (Subject to the trace-coverage caveats below — we deliberately avoid calling this "sound" without qualification.)
- **Extern crypto stubs are trusted as axioms:** `hashTreeRoot`, `blsVerify`, etc. are accepted at their recorded results and not re-executed.

**What is being proven, precisely:** the proof binds the IR-program hash, the input hash, the output hash, and the checked trace semantics above. It does **not** establish equivalence to Lean's own runtime, and it does **not** establish correctness of the stubbed crypto. The trust boundary is exactly the set of extern stubs plus the trace-coverage gaps described in "Honest caveats."

## Benchmark results

### Three-approach comparison

zkVM cycles and segments, ETH2 STF, N validators:

| Approach | N=10 cycles | N=10 seg | N=100 cycles | N=100 seg |
|----------|-------------|----------|--------------|-----------|
| Lean (compiled, incl. `Init`) | 26,148,291 | 29 | 35,281,299 | 38 |
| Rust (compiled, baseline) | 12,491,509 | 13 | 14,446,747 | 15 |
| **IR Trace (zkVM verify)** | **N/A — blocked by trace size** | **N/A** | **N/A** | **N/A** |

The IR Trace row is **not** blank because it's small — it is **unmeasurable** at present: the trace cannot be fed to the guest (see "The wall" below). Do not read it as comparable to the Lean/Rust cycle counts.

### IR Trace — host interpreter (these are host stats, not zkVM cycles)

The numbers below are **host wall-clock time and step counts** from interpreting the IR. They are *not* zkVM cycles and should not be compared to the guest-cycle table above.

N=10 (median of 3 runs):

```
Wall time:    7.71s median  (7.62s min, 11.89s max — run 1 is cold-cache)
Trace steps:  238,049
  PrimResult:   100,490  (42.2%)
  Call:          51,128  (21.5%)
  ProjResult:    39,701  (16.7%)
  Branch:        27,111  (11.4%)
  CtorCreate:    10,865  ( 4.6%)
  SetResult:      8,754  ( 3.7%)
Value table:  639,836 entries
Output:       78,522 bytes (Success)
```

N=100 (median of 3 runs):

```
Wall time:    11.19s median  (11.18s min, 16.44s max)
Trace steps:  324,741
  PrimResult:   157,376  (48.5%)
  Call:          62,832  (19.3%)
  ProjResult:    46,905  (14.4%)
  Branch:        33,685  (10.4%)
  CtorCreate:    13,835  ( 4.3%)
  SetResult:     10,108  ( 3.1%)
Value table:  867,320 entries
Output:       91,752 bytes (Success)
```

Scaling from N=10 → N=100:

| Metric | N=10 | N=100 | Ratio |
|--------|------|-------|-------|
| Total steps | 238,049 | 324,741 | 1.36× |
| Wall time (median) | 7.71s | 11.19s | 1.45× |
| Value table entries | 639,836 | 867,320 | 1.36× |
| PrimResult steps | 100,490 | 157,376 | 1.57× |
| Output size | 78,522 B | 91,752 B | 1.17× |

`PrimResult` (arithmetic) scales the steepest, as expected from per-validator balance/epoch math.

A consistency note: IR Trace outputs are **224 bytes smaller** than the compiled Lean/Rust outputs at both N (78,522 B vs 78,746 B; 91,752 B vs 91,976 B). We attribute this to a difference in the test-input serializer used to drive the interpreter versus the compiled guests, not to a divergence in the STF itself — but we flag it as not-yet-reconciled.

### IR Trace — the `sum` example proves end-to-end

A toy `sum` program with scalar input works through the full pipeline:

```
Mode:         execute
Trace:        3,843 bytes (bincode)
User cycles:  653,173
Segments:     1
Wall time:    76.18ms
Output:       8 bytes (Success)
```

This is the existence proof that the architecture is sound in shape: tiny guest, host-generated trace, re-checked end-to-end.

## The wall: trace size

The ETH2 STF generates a trace on the host, but it is **8.14 GB** (serialized bincode), with **639,836** value-table entries. We cannot feed it to the guest: the input path in this implementation serializes a `Vec<u8>` via `env::write()` with a **u32 length prefix**, and the host hits a `TryFromIntError` at ~4 GB.

> We state this as the **input-path limit observed in this implementation**, not as an authoritative RISC Zero spec figure.

Why is the trace so large? The interpreter clones **every intermediate value** into the value table, and ETH2 intermediates include large `ByteArray`s (the beacon state is ~78 KB) and deeply nested `Object`s, many of them near-duplicates (e.g. a byte array modified at a single index becomes a full new copy). Switching the trace format from JSON (~15 GB) to bincode (8.14 GB) only bought ~1.8×, because the cost is dominated by serializing complex nested values, not by JSON syntax overhead.

Contrast with **compiled Lean**: there, intermediates live in paged zkVM memory and are never serialized. IR Trace externalizes all computation to the host and must hand every intermediate result to the guest — which is exactly what blows up.

## Honest caveats

This is a research prototype; some gaps matter for soundness and we want to be upfront:

1. **Not a faithful port of Lean's runtime.** Values, the stack/frame model, and native/extern dispatch are a Rust re-implementation that approximates Lean semantics for this workload, not Lean's actual runtime.
2. **Crypto is stubbed and trusted** (`hashTreeRoot`, `blsVerify`, …), as noted above.
3. **Trace coverage is incomplete for in-place mutations.** Be careful here, because it's easy to overstate what's checked: the `Set` field-update op *does* emit a re-checked `SetResult` step. But the in-place mutation ops `USet` / `SSet` / `SetTag` are **executed by the interpreter yet emit no trace step today**, so they currently fall **outside** the verified set. Closing this gap is required before any ETH2 proof would be meaningful.

## Where we'd like feedback

We've split this into engineering ideas (where we have a plan) and open research questions (where we don't).

**Engineering / compression ideas (to get the trace under the input limit):**

- **Value dedup + hash references** — replace repeated values with references to a single stored copy. Est. 2–5× on ETH2's heavy `ByteArray` reuse.
- **Delta-encoded `ByteArray`s** — store `{base_id, offset, patch}` instead of a full copy when a large array changes in a small region. Est. 10–50× for this workload.
- **Chunked / streaming verification** — split the trace into guest-sized chunks verified independently with hash chaining, removing the single-input size limit entirely (but requires redesigning the guest + driver).
- **Dead-value pruning** — drop value-table entries that no step or output references (liveness pass after trace generation).

**Open research questions:**

- Is there prior art on **trace compression for interpret-then-verify zkVM designs** (host interprets, guest checks)? We'd love pointers.
- Given the 8 GB wall, is **streaming verification** actually worth the complexity, or is the honest conclusion "just compile the spec to RISC-V"? Where does interpret-then-verify win in practice?
- What's the cleanest way to **close the trace-coverage gap** (`USet` / `SSet` / `SetTag`) so that in-place mutation is verifiable without a faithful Lean-runtime port?

## Reproduce

Repo: **[link to repo]**

```bash
# Extract Lean lambda-RC IR (requires the pinned Lean toolchain)
just dump-ir
just filter-ir

# Host interpreter benchmark across validator counts
just bench-ir-trace 10,100

# End-to-end zkVM execute path (sum example works; ETH2 is blocked by trace size)
just verify-ir-trace /tmp/eth2_input_10.bin
```

Feedback, prior art, and "you're holding it wrong" corrections all very welcome.
