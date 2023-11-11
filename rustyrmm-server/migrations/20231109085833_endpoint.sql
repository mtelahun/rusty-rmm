-- Add migration script here
CREATE DOMAIN AssetId AS UUID;
CREATE DOMAIN SystemUuid AS UUID;
CREATE DOMAIN MachineId AS TEXT;
CREATE TYPE RegistrationState AS ENUM('new', 'upd');

CREATE TABLE endpoint (
    id AssetId NOT NULL,
    system_id MachineId,
    hostname TEXT,
    reg_state RegistrationState NOT NULL,
    PRIMARY KEY(id)
);

CREATE TABLE os_info (
    id AssetId NOT NULL,
    full_name TEXT,
    family TEXT,
    version_ TEXT,
    vert_system TEXT,
    vert_role TEXT,
    tz TEXT,
    PRIMARY KEY(id)
);