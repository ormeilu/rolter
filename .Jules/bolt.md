
## Redis Pipeline Optimizations

- Replaced sequential `INCR` and `EXPIRE` Redis calls inside loops with `redis::pipe()` in `crates/rolter-gateway/src/rate_limits.rs` for both request limits and token limits.
- Benchmarks showed a latency drop from ~35ms to ~1ms for 100 iterations of batched calls by avoiding round-trips.
