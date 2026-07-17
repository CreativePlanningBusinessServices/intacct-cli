use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{message}")]
    Api {
        status: u16,
        message: String,
        details: Vec<Value>,
        support_id: Option<String>,
    },
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    Auth(String),
    #[error("{0}")]
    Network(String),
}

impl CliError {
    pub fn exit_code(&self) -> u8 {
        match self {
            CliError::Api { .. } => 1,
            CliError::Usage(_) => 2,
            CliError::Auth(_) => 3,
            CliError::Network(_) => 4,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            CliError::Api {
                status,
                message,
                details,
                support_id,
            } => {
                let mut envelope = json!({"kind": "api", "status": status, "message": message, "details": details});
                if let Some(id) = support_id {
                    envelope["supportId"] = json!(id);
                }
                envelope
            }
            CliError::Usage(message) => json!({"kind": "usage", "message": message}),
            CliError::Auth(message) => json!({"kind": "auth", "message": message}),
            CliError::Network(message) => json!({"kind": "network", "message": message}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_error_kinds() {
        assert_eq!(
            CliError::Api {
                status: 400,
                message: "bad".into(),
                details: vec![],
                support_id: None
            }
            .exit_code(),
            1
        );
        assert_eq!(CliError::Usage("no account".into()).exit_code(), 2);
        assert_eq!(CliError::Auth("expired".into()).exit_code(), 3);
        assert_eq!(CliError::Network("timeout".into()).exit_code(), 4);
    }

    #[test]
    fn api_error_serializes_full_envelope_with_support_id() {
        let error = CliError::Api {
            status: 422,
            message: "operationFailed: Employee Contact info cannot be empty".into(),
            details: vec![
                json!({"code": "BL01001973", "message": "Employee Contact info cannot be empty"}),
            ],
            support_id: Some("sojLj~X2vg0DE3Vop0PJU6UeQVxQAAAEw".into()),
        };
        assert_eq!(
            error.to_json(),
            json!({
                "kind": "api", "status": 422,
                "message": "operationFailed: Employee Contact info cannot be empty",
                "details": [{"code": "BL01001973", "message": "Employee Contact info cannot be empty"}],
                "supportId": "sojLj~X2vg0DE3Vop0PJU6UeQVxQAAAEw",
            })
        );
    }

    #[test]
    fn api_error_omits_support_id_when_absent() {
        let error = CliError::Api {
            status: 404,
            message: "not found".into(),
            details: vec![],
            support_id: None,
        };
        assert_eq!(
            error.to_json(),
            json!({"kind": "api", "status": 404, "message": "not found", "details": []})
        );
    }

    #[test]
    fn auth_error_serializes_without_status() {
        assert_eq!(
            CliError::Auth("refresh expired".into()).to_json(),
            json!({"kind": "auth", "message": "refresh expired"})
        );
    }
}
