-- The Anthropic and OpenAI keys belong to the install, not to a client org, so they are
-- stored with `org_id NULL`. SQLite treats NULLs in a PRIMARY KEY as distinct, so
-- `(org_id, name)` does not constrain those rows at all and `INSERT OR REPLACE` would
-- pile up a second copy of every global key instead of replacing the first. This index
-- is what makes one name mean one global secret.
CREATE UNIQUE INDEX IF NOT EXISTS secrets_global ON secrets (name) WHERE org_id IS NULL;
