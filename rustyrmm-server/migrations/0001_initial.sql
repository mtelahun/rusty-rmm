-- Initial schema. All ids are Server-generated UUIDv7.
-- Secrets (session tokens, Pairing Tokens) are stored only as SHA-256 hashes.

CREATE TABLE operators (
    id            uuid PRIMARY KEY,
    username      text NOT NULL UNIQUE,
    password_hash text NOT NULL, -- argon2 PHC string
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE operator_sessions (
    token_hash  bytea PRIMARY KEY,
    operator_id uuid NOT NULL REFERENCES operators (id) ON DELETE CASCADE,
    created_at  timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz NOT NULL,
    revoked_at  timestamptz
);

CREATE TABLE environments (
    id         uuid PRIMARY KEY,
    name       text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE pairing_tokens (
    id             uuid PRIMARY KEY,
    environment_id uuid NOT NULL REFERENCES environments (id),
    secret_hash    bytea NOT NULL UNIQUE,
    created_at     timestamptz NOT NULL DEFAULT now(),
    expires_at     timestamptz NOT NULL,
    revoked_at     timestamptz
);
CREATE INDEX pairing_tokens_environment ON pairing_tokens (environment_id);

CREATE TABLE endpoints (
    id             uuid PRIMARY KEY,
    environment_id uuid NOT NULL REFERENCES environments (id),
    hostname       text NOT NULL,
    paired_at      timestamptz NOT NULL DEFAULT now(),
    last_seen_at   timestamptz, -- written about once a minute; Online status lives in memory
    revoked_at     timestamptz
);
CREATE INDEX endpoints_environment ON endpoints (environment_id);

CREATE TABLE agent_certificates (
    serial      bytea PRIMARY KEY,
    endpoint_id uuid NOT NULL REFERENCES endpoints (id) ON DELETE CASCADE,
    not_before  timestamptz NOT NULL,
    not_after   timestamptz NOT NULL,
    issued_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX agent_certificates_endpoint ON agent_certificates (endpoint_id);

-- One row per change; kept 1 year. Stored as the encoded common.v1.Inventory message.
CREATE TABLE inventories (
    id           bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    endpoint_id  uuid NOT NULL REFERENCES endpoints (id) ON DELETE CASCADE,
    collected_at timestamptz NOT NULL, -- Agent clock
    received_at  timestamptz NOT NULL, -- Server clock
    inventory    bytea NOT NULL
);
CREATE INDEX inventories_endpoint_received ON inventories (endpoint_id, received_at DESC);

-- Raw 30s Samples, kept 7 days.
CREATE TABLE samples (
    endpoint_id        uuid NOT NULL REFERENCES endpoints (id) ON DELETE CASCADE,
    collected_at       timestamptz NOT NULL, -- Agent clock
    received_at        timestamptz NOT NULL, -- Server clock
    cpu_usage_percent  real NOT NULL,
    memory_used_bytes  bigint NOT NULL,
    memory_total_bytes bigint NOT NULL,
    disks              jsonb NOT NULL,
    interfaces         jsonb NOT NULL
);
CREATE INDEX samples_endpoint_collected ON samples (endpoint_id, collected_at);
CREATE INDEX samples_received ON samples (received_at);

-- 5-minute rollups of Samples, kept 90 days.
CREATE TABLE sample_rollups (
    endpoint_id           uuid NOT NULL REFERENCES endpoints (id) ON DELETE CASCADE,
    bucket_start          timestamptz NOT NULL,
    cpu_usage_percent_avg real NOT NULL,
    cpu_usage_percent_max real NOT NULL,
    memory_used_bytes_avg bigint NOT NULL,
    memory_used_bytes_max bigint NOT NULL,
    disks                 jsonb NOT NULL,
    interfaces            jsonb NOT NULL,
    PRIMARY KEY (endpoint_id, bucket_start)
);

-- Kept 1 year. No foreign key to endpoints: Commands survive DeleteEndpoint.
CREATE TABLE commands (
    id                 uuid PRIMARY KEY,
    endpoint_id        uuid NOT NULL,
    endpoint_hostname  text NOT NULL,
    operator_username  text NOT NULL,
    interpreter        smallint NOT NULL, -- common.v1.Interpreter
    script             text NOT NULL,
    timeout_seconds    integer NOT NULL,
    status             smallint NOT NULL, -- operator.v1.CommandStatus
    created_at         timestamptz NOT NULL DEFAULT now(),
    exit_code          integer,
    stdout             bytea,
    stderr             bytea,
    truncated          boolean,
    started_at         timestamptz,
    finished_at        timestamptz
);
CREATE INDEX commands_endpoint_created ON commands (endpoint_id, created_at DESC);
CREATE INDEX commands_created ON commands (created_at);
