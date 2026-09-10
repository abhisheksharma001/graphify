-- A job's org, in a column rather than inside its request.
--
-- `jobs` holds a call's words — the request naming it, the model's verdicts with the
-- sentence it quoted, every line the brain printed — and no `call_id`, so retention could
-- not reach it. Retention is per-org, and the org was written where only a parser could
-- read it: `create_job`'s caller builds `{"org": …, "body": …}` and stores the whole thing
-- as `input`. The backfill is that same fact moved, not a guess about it — every row this
-- code has ever written names its org there.
ALTER TABLE jobs ADD COLUMN org_id INTEGER;

-- `json_valid` first, and not because a row is expected to fail it: `json_extract` over
-- text that is not JSON raises, a raise inside a migration fails the migration, and a
-- migration that fails runs again on the next open and fails again. The cost of the
-- unguarded version is a database that never opens; the cost of this one is a row that
-- keeps the retention it has always had, which is none.
UPDATE jobs
   SET org_id = json_extract(input, '$.org')
 WHERE org_id IS NULL AND json_valid(input);

CREATE INDEX idx_jobs_org_created ON jobs (org_id, created_at);
