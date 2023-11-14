-- Add migration script here
CREATE DOMAIN AssetId AS UUID;
CREATE DOMAIN MachineId AS TEXT;
CREATE DOMAIN CpuId as INTEGER;
CREATE DOMAIN DiskId as INTEGER;
CREATE TYPE RegistrationState AS ENUM('new', 'upd');
CREATE TYPE DiskType AS ENUM('unknown', 'hdd', 'ssd');

CREATE TABLE endpoint (
    id AssetId NOT NULL,
    system_id MachineId,
    hostname TEXT,
    reg_state RegistrationState NOT NULL,
    PRIMARY KEY(id)
);

CREATE TABLE os_info (
    id AssetId NOT NULL,
    os_name TEXT,
    os_ver TEXT,
    kernel_ver TEXT,
    virt_system TEXT,
    virt_role TEXT,
    tz TEXT,
    PRIMARY KEY(id),
    CONSTRAINT fk_endpoint_id
        FOREIGN KEY(id)
            REFERENCES endpoint(id)
);

CREATE TABLE cpu_info (
    id AssetId NOT NULL,
    core_count INTEGER NOT NULL,
    thread_count INTEGER NOT NULL,
    PRIMARY KEY(id),
    CONSTRAINT fk_endpoint_id
        FOREIGN KEY(id)
            REFERENCES endpoint(id)
);

CREATE TABLE cpu (
    cpu_info_id AssetId NOT NULL,
    cpu_id CpuId NOT NULL,
    name TEXT,
    vendor_id TEXT,
    brand TEXT,
    frequency TEXT,
    PRIMARY KEY(cpu_info_id, cpu_id),
    CONSTRAINT fk_cpu_info_id
        FOREIGN KEY(cpu_info_id)
            REFERENCES cpu_info(id)
);

CREATE TABLE mem_info (
    id AssetId NOT NULL,
    total BIGINT NOT NULL,
    used BIGINT NOT NULL,
    PRIMARY KEY(id),
    CONSTRAINT fk_endpoint_id
        FOREIGN KEY(id)
            REFERENCES endpoint(id)
);

CREATE TABLE disk_info (
    id AssetId NOT NULL,
    PRIMARY KEY(id),
    CONSTRAINT fk_endpoint_id
        FOREIGN KEY(id)
            REFERENCES endpoint(id)
);

CREATE TABLE disk (
    disk_info_id AssetId NOT NULL,
    disk_id DiskId NOT NULL,
    disk_name TEXT,
    disk_type DiskType NOT NULL,
    filesystem TEXT,
    mount_point TEXT,
    total_size NUMERIC(20,0) NOT NULL,
    free NUMERIC(20,0) NOT NULL,
    PRIMARY KEY(disk_info_id, disk_id),
    CONSTRAINT fk_disk_info_id
        FOREIGN KEY(disk_info_id)
            REFERENCES disk_info(id)
);
