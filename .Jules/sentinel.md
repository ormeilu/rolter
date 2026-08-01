## 2026-08-01 - [ClickHouse Parameter Mismatch Vulnerability]
**Vulnerability:** Found a mismatch in a ClickHouse parameterized query where the SQL template string defined the parameter as `{event_id:String}` but the actual parameter passed to the query execution was bound as `param_event_id`.
**Learning:** In ClickHouse queries executed over the HTTP interface using the `reqwest` client in this codebase, URL query string parameterization relies on matching the parameter name bound in the Rust code (e.g., `param_event_id`) directly to the placeholder within the SQL string. A mismatch doesn't correctly substitute the parameter.
**Prevention:** Always verify that the placeholder variable name in the ClickHouse SQL string directly matches the parameter name defined in the Rust code.
