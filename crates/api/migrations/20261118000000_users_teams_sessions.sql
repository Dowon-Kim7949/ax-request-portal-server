-- Teams, users and sign-in sessions.
--
-- Teams and users are never deleted. Requests, drafts, notifications and AI
-- usage will point at them, so a person is deactivated (users.active) and a
-- team is renamed instead. Later tables reference both with ON DELETE RESTRICT.
--
-- AI usage rows must copy the caller's team_id at call time: someone who moves
-- to another team leaves their past usage with the old team, so usage reports
-- never join users.team_id for history.

create table teams (
    id         uuid primary key default gen_random_uuid(),
    -- Unique within the organisation, ignoring case (contract: Team.name).
    name       text not null check (name = btrim(name) and char_length(name) between 1 and 40),
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create unique index teams_name_key on teams (lower(name));

create table users (
    id            uuid primary key default gen_random_uuid(),
    -- Kept as entered; sign-in compares lower(email), like the frontend's mock API.
    email         text not null check (email <> ''),
    name          text not null check (btrim(name) <> ''),
    -- Operators are provisioned by the service (ax-api user create), never by an admin.
    role          text not null check (role in ('requester', 'executive', 'developer', 'admin', 'operator')),
    team_id       uuid references teams (id) on delete restrict,
    active        boolean not null default true,
    -- Argon2id in PHC string form. NULL: the account cannot sign in until a password is set.
    password_hash text,
    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now()
);

create unique index users_email_key on users (lower(email));
create index users_team_id_idx on users (team_id);

-- One row per sign-in. The bearer token itself is never stored, only its
-- SHA-256, so a copy of the database does not hand out live sessions.
create table sessions (
    id                  uuid primary key default gen_random_uuid(),
    user_id             uuid not null references users (id) on delete cascade,
    token_hash          bytea not null unique check (octet_length(token_hash) = 32),
    created_at          timestamptz not null default now(),
    last_seen_at        timestamptz not null default now(),
    -- Slides forward on use, but never past absolute_expires_at.
    expires_at          timestamptz not null,
    absolute_expires_at timestamptz not null,
    check (expires_at <= absolute_expires_at)
);

create index sessions_user_id_idx on sessions (user_id);
create index sessions_expires_at_idx on sessions (expires_at);
