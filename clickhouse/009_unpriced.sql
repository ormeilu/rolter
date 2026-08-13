-- A model with no price row was billed at zero and reported as zero, so
-- "this traffic cost nothing" and "we do not know what this traffic cost"
-- rendered identically -- and a fleet could run for a month showing $0.00
-- with nothing prompting a second look (#969).
--
-- Defaults to 0 so existing rows keep their meaning: everything recorded
-- before this column existed was written by a gateway that had a price, or
-- had none and we cannot now tell which. Treating history as priced is the
-- conservative reading -- it under-reports the unpriced share rather than
-- inventing one.

alter table request_logs
    add column if not exists unpriced UInt8 default 0;
