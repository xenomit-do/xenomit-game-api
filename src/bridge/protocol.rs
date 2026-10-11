use crate::{ApiError, Result};

/// Decode the outer line protocol (OK ... / ERR ...).
pub(crate) fn unwrap_reply(reply: &str) -> Result<String> {
    let line = reply.trim_end_matches(['\r', '\n']);
    if let Some(payload) = line.strip_prefix("OK ") {
        Ok(payload.to_owned())
    } else if let Some(message) = line.strip_prefix("ERR ") {
        Err(ApiError::Transport(message.to_owned()))
    } else {
        Err(ApiError::Protocol(format!(
            "malformed bridge reply: {line}"
        )))
    }
}

/// Command tokens are tab-delimited in the pipe protocol.
pub(crate) fn validate_token(value: &str, label: &str) -> Result<()> {
    if value.is_empty() {
        return Err(ApiError::InvalidCall(format!("{label} cannot be empty")));
    }
    if value.contains(['\t', '\r', '\n']) {
        return Err(ApiError::InvalidCall(format!(
            "{label} cannot contain tabs or line breaks"
        )));
    }
    Ok(())
}
