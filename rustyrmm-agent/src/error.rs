#[derive(Debug)]
pub enum AgentError {
    Internal(String),
    Rpc(tonic::Status),
    Transport(tonic::transport::Error),
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            AgentError::Internal(s) => format!("internal error: {s}"),
            AgentError::Rpc(s) => format!("rpc failed: {s}"),
            AgentError::Transport(e) => format!("transport error: {e}"),
        };

        write!(f, "{}", msg)
    }
}
impl std::error::Error for AgentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AgentError::Rpc(e) => Some(e),
            AgentError::Transport(e) => Some(e),
            _ => None,
        }
    }
}

impl From<tonic::Status> for AgentError {
    fn from(value: tonic::Status) -> Self {
        AgentError::Rpc(value)
    }
}

impl From<tonic::transport::Error> for AgentError {
    fn from(value: tonic::transport::Error) -> Self {
        AgentError::Transport(value)
    }
}
