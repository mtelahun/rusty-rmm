-- Set when the Agent killed the Command's process tree at its timeout.
ALTER TABLE commands ADD COLUMN timed_out boolean;
