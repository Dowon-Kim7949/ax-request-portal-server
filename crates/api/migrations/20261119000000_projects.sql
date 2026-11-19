-- Projects: what a request targets. An admin (project:manage) keeps them.
--
-- Owners are the developers responsible for a project; their order matters,
-- because a new request is assigned to the first one. Change targets (screens
-- or features a requester picks) and the rules every change must keep are
-- small lists that are always read and written whole, so they are JSONB in the
-- contract's own shape ([{ id, name, wikiDoc? }], [{ id, text, reference? }]).
-- Requests will refer to target ids, which therefore never change.

create table projects (
    id                  uuid primary key default gen_random_uuid(),
    name                text not null check (name = btrim(name) and name <> ''),
    description         text not null,
    icon                text not null check (icon in ('globe', 'smartphone', 'layout-dashboard', 'server', 'box')),
    repository_provider text not null check (repository_provider in ('gitlab', 'github')),
    repository_url      text not null,
    wiki_path           text not null check (wiki_path <> ''),
    -- Filled by the wiki sync of a later step; 0/0 until then.
    wiki_verified_docs  integer not null default 0 check (wiki_verified_docs >= 0),
    wiki_total_docs     integer not null default 0 check (wiki_total_docs >= 0),
    wiki_synced_at      timestamptz,
    targets             jsonb not null default '[]' check (jsonb_typeof(targets) = 'array'),
    -- The contract's `constraints`.
    rules               jsonb not null default '[]' check (jsonb_typeof(rules) = 'array'),
    active              boolean not null default true,
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now()
);

create table project_owners (
    project_id uuid not null references projects (id) on delete cascade,
    user_id    uuid not null references users (id) on delete restrict,
    position   smallint not null check (position >= 0),
    primary key (project_id, user_id),
    unique (project_id, position)
);

create index project_owners_user_id_idx on project_owners (user_id);
