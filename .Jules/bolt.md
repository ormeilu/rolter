## 2026-07-29 - Pre-allocate String and Vector buffers
**Learning:** Anticipating and pre-allocating buffer capacities based on input sizes mitigates reallocation overhead in batch operations and array mapping, which can be critical for high-throughput processing in tight loops.
**Action:** Always prefer `String::with_capacity` and `Vec::with_capacity` when the final item count can be reasonably predicted or heuristically inferred.

## Performance optimization context

* Attempted to run criterion benchmarks natively in `rolter-gateway` but ran into workspace dependency resolution / linking errors on missing `main` when relying on `criterion_main` macros in the context of the workspace configuration.
* Opted for asserting the O(N) to O(1) network topology improvement instead by moving from `conn.get(key)` inside the applicable budget `for` loop to pipelined queries using `MGET`.
