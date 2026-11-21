-- The Admin group, and which Operators belong to which Environments.
ALTER TABLE operators ADD COLUMN is_admin boolean NOT NULL DEFAULT false;

CREATE TABLE environment_members (
    environment_id uuid NOT NULL REFERENCES environments (id) ON DELETE CASCADE,
    operator_id    uuid NOT NULL REFERENCES operators (id) ON DELETE CASCADE,
    created_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (environment_id, operator_id)
);
CREATE INDEX environment_members_operator_id ON environment_members (operator_id);

-- Until now every Operator reached every Environment: keep that for existing ones.
UPDATE operators SET is_admin = true;
INSERT INTO environment_members (environment_id, operator_id)
SELECT e.id, o.id FROM environments e CROSS JOIN operators o;
