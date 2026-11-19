-- The operator's AI settings and the ledger of AI usage.
--
-- ai_config holds one JSON document per setting: `budget` (BudgetLimits),
-- `selections` (the model per task and the per-project overrides) and
-- `pricing` (AiPricing). A missing row means the template's default. Writers
-- take one advisory lock, so a check across settings (a price removed while a
-- task still uses the model) cannot race another write.
--
-- ai_usage books one row per model call or coding-agent run, priced when it
-- is booked: a later price change never rewrites it. It copies the caller's
-- team at call time, and is never deleted.

create table ai_config (
    name       text primary key check (name in ('budget', 'selections', 'pricing')),
    value      jsonb not null check (jsonb_typeof(value) = 'object'),
    updated_at timestamptz not null default now()
);

create table ai_usage (
    seq                bigint generated always as identity primary key,
    id                 text not null unique,
    at                 timestamptz not null default now(),
    task               text not null check (task in ('precheck', 'report', 'work')),
    provider           text not null check (provider in ('anthropic', 'openai')),
    model              text not null,
    runtime            text not null check (runtime in ('api', 'claude_code', 'codex')),
    request_id         bigint references requests (id) on delete restrict,
    user_id            uuid not null references users (id) on delete restrict,
    team_id            uuid references teams (id) on delete restrict,
    input_tokens       bigint not null check (input_tokens >= 0),
    cache_read_tokens  bigint not null check (cache_read_tokens >= 0),
    cache_write_tokens bigint not null check (cache_write_tokens >= 0),
    output_tokens      bigint not null check (output_tokens >= 0),
    reasoning_tokens   bigint check (reasoning_tokens >= 0),
    -- Rounded to 1/100 when booked, as the frontend's bookUsage does.
    cost_usd           double precision not null check (cost_usd >= 0),
    cost_krw           double precision not null check (cost_krw >= 0)
);

create index ai_usage_at_idx on ai_usage (at);
create index ai_usage_request_id_idx on ai_usage (request_id) where request_id is not null;
