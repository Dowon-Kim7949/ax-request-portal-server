-- A runner's claim on an AI job.
--
-- A separate runner (`ax-api runner`) takes jobs over HTTP. Its claim carries
-- a token its calls must show, and a lease it renews while it works: a job
-- whose runner falls silent past the lease goes back to the pool as another
-- attempt, and a late answer under an old claim is refused. Jobs the server
-- runs itself have neither.

alter table ai_jobs add column claim uuid;
alter table ai_jobs add column lease_until timestamptz;
