-- Email as a second Notification channel: per-recipient channels and a digest outbox.
-- Existing recipients stay dashboard-only.
ALTER TABLE environment_alert_recipients
    ADD COLUMN dashboard boolean NOT NULL DEFAULT true,
    ADD COLUMN email boolean NOT NULL DEFAULT false,
    ADD CONSTRAINT environment_alert_recipients_channel CHECK (dashboard OR email);

-- Digest emails waiting to be sent; a row is deleted once sent or given up on.
CREATE TABLE email_outbox (
    id              uuid PRIMARY KEY,
    operator_id     uuid NOT NULL REFERENCES operators (id) ON DELETE CASCADE,
    subject         text NOT NULL,
    body            text NOT NULL,
    attempts        integer NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL DEFAULT now(),
    created_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX email_outbox_due ON email_outbox (next_attempt_at);
