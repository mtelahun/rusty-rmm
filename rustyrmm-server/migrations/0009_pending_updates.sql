-- The Updates currently pending on each Endpoint, replaced by every known list the Agent sends.
-- first_seen_at survives version bumps; rows are deleted once an Update is no longer pending.
CREATE TABLE pending_updates (
    endpoint_id       uuid NOT NULL REFERENCES endpoints (id) ON DELETE CASCADE,
    install_key       text NOT NULL,
    title             text NOT NULL,
    security          boolean, -- null = unknown
    installed_version text,
    new_version       text NOT NULL,
    severity          text,
    advisory_id       text,
    first_seen_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (endpoint_id, install_key)
);

-- When the OS last refreshed its package metadata, as of the latest known list; null = unknown.
ALTER TABLE endpoints ADD COLUMN update_metadata_refreshed_at timestamptz;
