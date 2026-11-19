-- Saved drafts: half-written requests kept on the server to finish later.
--
-- `content` is the contract's DraftContent as the client sent it (unknown
-- fields dropped). Its attachment ids are checked when it is saved, not tied
-- to the attachments table. `client_id` makes a repeated create return the
-- draft the first one made; `seq` orders drafts saved at the same instant.

create table drafts (
    seq        bigint generated always as identity primary key,
    id         text not null unique,
    owner_id   uuid not null references users (id) on delete restrict,
    client_id  text,
    step       integer not null check (step between 0 and 6),
    content    jsonb not null check (jsonb_typeof(content) = 'object'),
    version    bigint not null default 1 check (version >= 1),
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    unique (owner_id, client_id)
);

create index drafts_owner_id_idx on drafts (owner_id, updated_at desc, seq desc);
