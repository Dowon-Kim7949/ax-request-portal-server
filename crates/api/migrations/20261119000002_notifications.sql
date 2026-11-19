-- In-app notifications and each person's notification settings.
--
-- A notification is stored as a message key and its parameters, and written
-- out in the reader's language when listed, as the frontend's mock API does.
-- Each person keeps at most 200; older ones are dropped as new ones arrive.

create table notifications (
    seq        bigint generated always as identity primary key,
    id         uuid not null unique default gen_random_uuid(),
    user_id    uuid not null references users (id) on delete restrict,
    request_id bigint references requests (id) on delete cascade,
    key        text not null,
    params     jsonb not null default '{}' check (jsonb_typeof(params) = 'object'),
    created_at timestamptz not null default now(),
    read_at    timestamptz
);

create index notifications_user_id_idx on notifications (user_id, seq desc);

-- A person without a row has the defaults: in-app and e-mail on, messenger
-- off, every notification (not only those asking for action).
create table notification_settings (
    user_id              uuid primary key references users (id) on delete restrict,
    in_app               boolean not null,
    email                boolean not null,
    messenger            boolean not null,
    only_action_required boolean not null,
    updated_at           timestamptz not null default now()
);
