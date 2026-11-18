//! Generated gRPC types and services for RustyRMM. Sources live in `proto/`.

pub mod common {
    pub mod v1 {
        tonic::include_proto!("rustyrmm.common.v1");
    }
}

pub mod agent {
    pub mod v1 {
        tonic::include_proto!("rustyrmm.agent.v1");
    }
}

pub mod operator {
    pub mod v1 {
        tonic::include_proto!("rustyrmm.operator.v1");
    }
}

#[cfg(test)]
mod tests {
    use super::agent::v1::{agent_message, AgentMessage};
    use super::common::v1::{Sample, UpdateStatus};
    use prost::Message;

    #[test]
    fn agent_message_round_trips_across_packages() {
        let msg = AgentMessage {
            message: Some(agent_message::Message::Sample(Sample {
                cpu_usage_percent: 12.5,
                memory_used_bytes: 1 << 30,
                ..Default::default()
            })),
        };
        let decoded = AgentMessage::decode(msg.encode_to_vec().as_slice()).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn unknown_update_count_is_absent_not_zero() {
        let status = UpdateStatus {
            regular: Some(3),
            security: None,
        };
        let decoded = UpdateStatus::decode(status.encode_to_vec().as_slice()).unwrap();
        assert_eq!(decoded.security, None);
        assert_eq!(decoded.regular, Some(3));
    }
}
