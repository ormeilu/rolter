# Performance optimization context

* Attempted to run criterion benchmarks natively in `rolter-gateway` but ran into workspace dependency resolution / linking errors on missing `main` when relying on `criterion_main` macros in the context of the workspace configuration.
* Opted for asserting the O(N) to O(1) network topology improvement instead by moving from `conn.get(key)` inside the applicable budget `for` loop to pipelined queries using `MGET`.
