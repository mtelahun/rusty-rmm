-- Update Installs: like commands, no foreign key to endpoints, so the record outlives a
-- deleted Endpoint and its hostname is copied.
CREATE TABLE update_installs (
    id                uuid PRIMARY KEY,
    endpoint_id       uuid NOT NULL,
    endpoint_hostname text NOT NULL,
    operator_username text NOT NULL,
    selection         smallint NOT NULL, -- common.v1.UpdateSelection
    reboot            smallint NOT NULL, -- common.v1.RebootPolicy
    status            smallint NOT NULL, -- operator.v1.UpdateInstallStatus
    created_at        timestamptz NOT NULL DEFAULT now(),
    started_at        timestamptz,
    finished_at       timestamptz,
    -- [{"package": ..., "status": common.v1.UpdateResultStatus, "exit_code": ...}]
    results           jsonb NOT NULL DEFAULT '[]',
    -- The Agent's Update counts after the install (null: unknown).
    updates_regular   integer,
    updates_security  integer
);
CREATE INDEX update_installs_endpoint_created ON update_installs (endpoint_id, created_at DESC);
-- One unfinished (Requested or Running) Update Install per Endpoint.
CREATE UNIQUE INDEX update_installs_one_unfinished ON update_installs (endpoint_id)
    WHERE status IN (1, 2);
