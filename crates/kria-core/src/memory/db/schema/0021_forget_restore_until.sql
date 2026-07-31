-- F1.7.2: Add `restore_until` column to `memories` for the governed Forget
-- lifecycle (design §5.4 "Lifecycle and erasure truth", MGR-040).
--
-- When a memory is transitioned to the `Forgotten` Truth_State via the governed
-- `Lifecycle::forget()` commit (task F1.7.2), this column is set to
-- `now() + 30 days` (RFC3339 UTC text). A `NULL` value means the memory has
-- never been forgotten; a non-NULL value that is in the past means the restore
-- window has expired.
--
-- SQLite's additive-only migration policy: this is an `ALTER TABLE ... ADD
-- COLUMN` with a DEFAULT of NULL so all existing rows are back-filled with NULL
-- without touching their data.

ALTER TABLE memories ADD COLUMN restore_until TEXT;
