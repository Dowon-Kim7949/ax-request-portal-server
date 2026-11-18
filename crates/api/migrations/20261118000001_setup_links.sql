-- One-time links to set or reset a password.
--
-- An admin makes one for a person (POST /users/{id}/setup-link); the person
-- opens it and sets their password (POST /auth/setup). At most one per person:
-- making a new link replaces the old one. Using a link deletes it. As with
-- sessions, only the token's SHA-256 is stored.

create table setup_links (
    user_id    uuid primary key references users (id) on delete cascade,
    token_hash bytea not null unique check (octet_length(token_hash) = 32),
    created_at timestamptz not null default now(),
    expires_at timestamptz not null
);

create index setup_links_expires_at_idx on setup_links (expires_at);
