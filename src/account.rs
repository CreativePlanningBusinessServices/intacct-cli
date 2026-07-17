pub const DEFAULT_API_BASE: &str = "https://api.intacct.com/ia/api/v1";

/// Single global host — tenant selection travels in the OAuth username, not the URL.
/// INTACCT_API_BASE exists so live-smoke tests and local mocks can redirect the whole CLI.
pub fn api_base() -> String {
    std::env::var("INTACCT_API_BASE")
        .map(|base| base.trim_end_matches('/').to_string())
        .unwrap_or_else(|_| default_api_base())
}

pub fn default_api_base() -> String {
    DEFAULT_API_BASE.to_string()
}

pub fn token_url() -> String {
    format!("{}/oauth2/token", api_base())
}

pub fn authorize_url() -> String {
    format!("{}/oauth2/authorize", api_base())
}

pub fn revoke_url() -> String {
    format!("{}/oauth2/revoke", api_base())
}

/// Intacct embeds the tenant in the client-credentials username:
/// `userId@companyId` at top level, `userId@companyId|entityId` entity-scoped.
pub fn username_for(user_id: &str, company_id: &str, entity_id: Option<&str>) -> String {
    match entity_id {
        Some(entity) => format!("{user_id}@{company_id}|{entity}"),
        None => format!("{user_id}@{company_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_formats_tenant_and_optional_entity() {
        assert_eq!(
            username_for("api_user", "demoCompany", None),
            "api_user@demoCompany"
        );
        assert_eq!(
            username_for("api_user", "demoCompany", Some("Central Region")),
            "api_user@demoCompany|Central Region"
        );
    }

    #[test]
    fn api_base_defaults_to_production_host() {
        // Note: don't set INTACCT_API_BASE in this test — env is process-global.
        assert_eq!(default_api_base(), "https://api.intacct.com/ia/api/v1");
    }

    #[test]
    fn oauth_urls_hang_off_the_base() {
        assert!(token_url().ends_with("/oauth2/token"));
        assert!(authorize_url().ends_with("/oauth2/authorize"));
        assert!(revoke_url().ends_with("/oauth2/revoke"));
    }
}
