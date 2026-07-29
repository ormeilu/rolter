-- Governance attribution carried by the virtual key, so spend rolls up by
-- business unit and customer and not by tenancy alone (#539). Empty string
-- means the key was unattributed at request time.

alter table request_logs
    add column if not exists business_unit_id String default '';

alter table request_logs
    add column if not exists customer_id String default '';
