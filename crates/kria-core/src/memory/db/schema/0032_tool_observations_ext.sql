-- Migration 0032: Extend tool_observations with server_id and affected_records_json
-- (F3.7.2 — policy-safe rich facts storage).
--
-- Adds two columns to `tool_observations`:
--   * server_id TEXT          — the server/service hosting the tool (e.g. MCP server name).
--                               NULL means native/unknown server context.
--   * affected_records_json TEXT — JSON array of authority record IDs (from `records`
--                               table) affected by this invocation. NULL means no
--                               records were explicitly linked. Validated as a JSON
--                               array when non-NULL; stored as plain text (no FK) to
--                               avoid polymorphic FK complexity. AuthorityTx callers
--                               are responsible for supplying only authorized IDs.
--
-- Both columns are additive with DEFAULT NULL so existing rows are unaffected.

ALTER TABLE tool_observations ADD COLUMN server_id TEXT DEFAULT NULL;
ALTER TABLE tool_observations ADD COLUMN affected_records_json TEXT
    DEFAULT NULL
    CHECK (affected_records_json IS NULL OR json_valid(affected_records_json));

-- Index server_id for future capability_observations-style aggregations.
CREATE INDEX IF NOT EXISTS idx_tool_observations_server
    ON tool_observations (server_id)
    WHERE server_id IS NOT NULL;
