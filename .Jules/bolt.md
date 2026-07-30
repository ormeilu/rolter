## 2026-07-29 - Pre-allocate String and Vector buffers
**Learning:** Anticipating and pre-allocating buffer capacities based on input sizes mitigates reallocation overhead in batch operations and array mapping, which can be critical for high-throughput processing in tight loops.
**Action:** Always prefer `String::with_capacity` and `Vec::with_capacity` when the final item count can be reasonably predicted or heuristically inferred.
