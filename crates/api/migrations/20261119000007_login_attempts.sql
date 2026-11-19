-- Sign-in attempts per email, to stop password guessing.
--
-- Every attempt is counted before the password is checked, so parallel
-- guesses cannot slip past the limit; a successful sign-in deletes the row.
-- Keyed by the email as typed, in lower case, whether or not an account has
-- it, so the answer never tells which emails exist. A row whose window or
-- lock has passed starts over at the next attempt and is swept away later.

create table login_attempts (
    email_key    text primary key,
    attempts     integer not null check (attempts >= 1),
    first_at     timestamptz not null default now(),
    locked_until timestamptz
);

create index login_attempts_first_at_idx on login_attempts (first_at);
