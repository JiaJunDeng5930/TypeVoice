use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortError {
    pub code: String,
    pub message: String,
    pub raw: Option<String>,
}

impl PortError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code: code.to_string(),
            raw: Some(message.clone()),
            message,
        }
    }

    pub fn from_message(default_code: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        let code = parse_error_code(&message).unwrap_or_else(|| default_code.to_string());
        Self {
            code,
            raw: Some(message.clone()),
            message,
        }
    }

    pub fn raw_message(&self) -> &str {
        self.raw.as_deref().unwrap_or(&self.message)
    }
}

impl std::fmt::Display for PortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PortError {}

pub type PortResult<T> = std::result::Result<T, PortError>;

pub fn parse_error_code(message: &str) -> Option<String> {
    for token in message.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        if token.starts_with("E_") && token.len() > 2 {
            return Some(token.to_string());
        }
        if token.starts_with("HTTP_")
            && token.len() > 5
            && token[5..].chars().all(|c| c.is_ascii_digit())
        {
            return Some(token.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_error_exposes_code() {
        let err = PortError::new("E_TEST", "failed");

        assert_eq!(err.code, "E_TEST");
        assert_eq!(err.message, "failed");
        assert_eq!(err.raw.as_deref(), Some("failed"));
    }
}
