-- Alert severity, snooze, manual resolve and action notes.
-- severity: 1 = informational, 2 = warning, 3 = error. Existing Alerts become warnings.
ALTER TABLE alerts
    ADD COLUMN severity smallint NOT NULL DEFAULT 2 CHECK (severity BETWEEN 1 AND 3),
    ADD COLUMN snoozed_until timestamptz,
    ADD COLUMN resolved_by_hand boolean NOT NULL DEFAULT false;

CREATE TABLE alert_actions (
    id          uuid PRIMARY KEY,
    alert_id    uuid NOT NULL REFERENCES alerts (id) ON DELETE CASCADE,
    operator_id uuid NOT NULL REFERENCES operators (id) ON DELETE CASCADE,
    text        text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX alert_actions_alert ON alert_actions (alert_id, id DESC);
