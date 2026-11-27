//! RustyRMM Server. A library so other crates' tests (e.g. the Agent's) can run it in-process.

pub mod agent_service;
pub mod auth;
pub mod ca;
pub mod commands;
pub mod db;
pub mod email;
pub mod ingest;
pub mod jobs;
pub mod operator_service;
pub mod operators;
pub mod pairing;
pub mod sessions;
pub mod update_installs;
pub mod web_ui;
