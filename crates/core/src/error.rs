use serde::Serialize;

/// Error shape returned to the frontend by every command.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AppError {
    pub kind: String,
    pub message: String,
}

impl AppError {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        Self::new("Internal", format!("{e:#}"))
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::new("Database", e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::new("Internal", format!("json: {e}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::new("Io", e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_kind_and_message() {
        let e = AppError::new("NoSource", "Серии 8 пока нет");
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["kind"], "NoSource");
        assert_eq!(json["message"], "Серии 8 пока нет");
    }

    #[test]
    fn converts_anyhow_as_internal() {
        let e: AppError = anyhow::anyhow!("boom").into();
        assert_eq!(e.kind, "Internal");
        assert!(e.message.contains("boom"));
    }
}
