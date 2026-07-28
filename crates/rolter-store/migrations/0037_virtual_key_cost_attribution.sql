-- attribute virtual-key spend to a business unit and/or customer so usage
-- rolls up along the governance hierarchy instead of tenancy alone
alter table virtual_keys
    add column if not exists business_unit_id uuid references business_units (id) on delete set null,
    add column if not exists customer_id uuid references customers (id) on delete set null;

create index if not exists virtual_keys_business_unit_idx
    on virtual_keys (business_unit_id)
    where business_unit_id is not null;

create index if not exists virtual_keys_customer_idx
    on virtual_keys (customer_id)
    where customer_id is not null;
