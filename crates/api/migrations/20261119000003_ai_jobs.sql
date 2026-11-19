-- The AI's work queue: one job per feasibility report or work run.
--
-- A job is queued when a request reaches `reporting` (a report) or
-- `in_progress` (a work run), and done when the AI's event has moved the
-- request on, or when the request has moved on without it (cancelled). At most
-- one open job per request and task. Workers claim jobs with
-- FOR UPDATE SKIP LOCKED, so several server instances never run one twice.
-- `lang` is the language the AI writes in: that of the person whose action
-- queued the job.

create table ai_jobs (
    id          bigint generated always as identity primary key,
    request_id  bigint not null references requests (id) on delete cascade,
    task        text not null check (task in ('report', 'work')),
    lang        text not null check (lang in ('ko', 'en')),
    status      text not null default 'queued' check (status in ('queued', 'running', 'done', 'failed')),
    attempts    integer not null default 0,
    run_after   timestamptz not null default now(),
    created_at  timestamptz not null default now(),
    started_at  timestamptz,
    finished_at timestamptz,
    error       text
);

create unique index ai_jobs_one_open_idx on ai_jobs (request_id, task) where status in ('queued', 'running');
create index ai_jobs_queued_idx on ai_jobs (run_after, id) where status = 'queued';
