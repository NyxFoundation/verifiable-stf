---
title: Trace Optimization Report
last_updated: 2026-03-24
tags:
  - optimization
  - benchmark
  - ir-trace
---

# Trace Optimization Report

## Summary

Three optimizations reduced the ETH2 N=10 trace from **8.14 GB to 683 MB** (11.9x reduction), bringing it well within the zkVM's ~4 GB input limit.

| Metric | Before | After | Change |
|--------|--------|-------|--------|
| **Trace size (bincode)** | 8.14 GB | 683 MB | **-91.6%** (11.9x) |
| **Value table entries** | 639,836 | 40,649 | **-93.6%** (15.7x) |
| **Trace steps** | 238,049 | 186,921 | **-21.5%** |
| Median interpreter time | 7.71s | 8.37s | +8.6% |

## Optimizations Applied

### 1. Call Step Removal

`TraceStep::Call` was removed from the trace. The guest verifier never verified Call steps — they served only as function boundary markers, while correctness was already guaranteed by the sub-steps (PrimResult, Branch, etc.) within each function body.

- **Steps removed**: 51,128 (21.5% of total)
- **Value registrations avoided**: ~200K (args + result per Call)

### 2. Fingerprint-Based Value Deduplication

Replaced the append-only `Vec<Value>` with a `ValueRegistry` that uses fingerprint-based deduplication (`HashMap<u64, Vec<ValueId>>`). When a value is registered, its hash is computed and compared against existing entries. If an exact match exists, the existing `ValueId` is returned without cloning.

- **Value table reduction**: 639,836 → 40,649 entries (15.7x)
- This was the dominant optimization — most intermediate values were duplicates created by repeated field projections and primitive operations on the same data.

### 3. Dead Value Pruning

After trace generation, a compaction pass removes values not referenced by any trace step or the output. All `ValueId` references are remapped to a contiguous range.

- Removes values that were registered but never used in a verification step.
- Ownership is moved from the interpreter via `std::mem::take()` to avoid peak memory doubling.

## Defensive Verification Checks Added

The guest verifier now validates:
- `header.magic == TRACE_MAGIC`
- `header.value_count == value_table.len()`
- `header.step_count == steps.len()`
- `output_value_id < value_table.len()`
- All `ValueId` references in every step are within bounds
- `CtorCreate` scalar content (previously only length was checked)

## Semantic Preservation

All three optimizations preserve the proof's semantics:
- **Call removal**: Call steps performed no verification; sub-steps still verify all computation.
- **Deduplication**: Same values, same verification — just fewer copies in the table.
- **Pruning**: Unreferenced values were never checked by the verifier.

The Lean 4 STF correctness proof (IR program hash + input hash → verified computation → output hash) is fully preserved.

## Performance Notes

- Interpreter time increased ~8.6% (7.71s → 8.37s) due to fingerprint computation on the hot path.
- This overhead is acceptable given the 11.9x trace size reduction.
- If needed, switching to `ahash` could reduce the hashing overhead.

## Benchmark Environment

```
Entry: risc0_main_eth2
Input: 79,510 bytes (10 validators)
Runs: 3 (median reported)
Profile: release
```

### After Optimization (N=10, 3 runs)

```
Timing:     8.37s (median), 7.97s (min), 11.09s (max)

Trace Steps:  186,921
  Branch:        27,111 (14.5%)
  PrimResult:   100,490 (53.8%)
  CtorCreate:    10,865 (5.8%)
  ProjResult:    39,701 (21.2%)
  SetResult:      8,754 (4.7%)

Value table:  40,649 entries
Trace size:   683 MB (bincode)
Output:       78,522 bytes (Success)
```

## zkVM Status

| Format | Before | After | Status |
|--------|--------|-------|--------|
| bincode | 8.14 GB | **683 MB** | Within ~4 GB limit |

The ETH2 N=10 trace can now be passed to the zkVM guest for verification.
