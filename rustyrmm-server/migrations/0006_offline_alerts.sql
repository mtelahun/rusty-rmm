-- Offline alerting: per-Environment and per-Endpoint thresholds, Alerts, and Operator Notifications.
ALTER TABLE environments
    ADD COLUMN offline_threshold_minutes integer NOT NULL DEFAULT 15
        CHECK (offline_threshold_minutes BETWEEN 5 AND 43200);

CREATE TABLE environment_alert_recipients (
    environment_id uuid NOT NULL REFERENCES environments (id) ON DELETE CASCADE,
    operator_id    uuid NOT NULL REFERENCES operators (id) ON DELETE CASCADE,
    PRIMARY KEY (environment_id, operator_id)
);

ALTER TABLE endpoints
    ADD COLUMN alerts_enabled boolean NOT NULL DEFAULT true,
    ADD COLUMN offline_threshold_minutes integer
        CHECK (offline_threshold_minutes BETWEEN 5 AND 43200);

CREATE TABLE alerts (
    id           uuid PRIMARY KEY,
    endpoint_id  uuid NOT NULL REFERENCES endpoints (id) ON DELETE CASCADE,
    -- The Endpoint's last_seen_at when the Alert opened, for the back-online duration.
    last_seen_at timestamptz NOT NULL,
    opened_at    timestamptz NOT NULL DEFAULT now(),
    resolved_at  timestamptz
);
CREATE UNIQUE INDEX alerts_one_open_per_endpoint ON alerts (endpoint_id) WHERE resolved_at IS NULL;

-- No foreign key to endpoints: Notifications outlive a deleted Endpoint, so hostname is copied.
-- kind: 1 = went offline, 2 = back online.
CREATE TABLE notifications (
    id              uuid PRIMARY KEY,
    operator_id     uuid NOT NULL REFERENCES operators (id) ON DELETE CASCADE,
    environment_id  uuid NOT NULL REFERENCES environments (id) ON DELETE CASCADE,
    hostname        text NOT NULL,
    kind            smallint NOT NULL,
    last_seen_at    timestamptz NOT NULL,
    offline_seconds bigint,
    created_at      timestamptz NOT NULL DEFAULT now(),
    read_at         timestamptz
);
CREATE INDEX notifications_operator_created ON notifications (operator_id, created_at DESC);
