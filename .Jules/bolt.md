## Redis MGET Batching
When iterating over limits or lists and fetching Redis keys for each item inside a loop (like `windowed_count` being awaited in a loop), we incur an N+1 query problem resulting in N roundtrips to Redis.
To solve this, we can pre-collect all the keys required into a `Vec<String>`, perform a single `conn.mget(&keys).await`, and then iterate over the results. This significantly reduced latency (from 5.9ms to 3.2ms in a local benchmark of 6 rate limits).
