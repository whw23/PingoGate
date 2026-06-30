//! Authorization boundary (constitution XX).
//!
//! Every privileged action — proxying a request and every Admin API call —
//! crosses [`AuthContext::authorize`]. There is no global-token equality
//! shortcut: even bootstrap admin credentials are turned into a [`Principal`]
//! and then authorized through this same boundary.

use crate::error::AppError;

/// The category of an authenticated caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalKind {
    /// A client presenting a named gateway key (data path).
    GatewayKey,
    /// An operator/automation calling the Admin API.
    Admin,
    /// The gateway itself (internal/system actions).
    System,
}

/// An authenticated caller identity.
#[derive(Debug, Clone)]
pub struct Principal {
    pub id: String,
    pub kind: PrincipalKind,
}

impl Principal {
    pub fn gateway_key(id: impl Into<String>) -> Self {
        Self { id: id.into(), kind: PrincipalKind::GatewayKey }
    }
    pub fn admin(id: impl Into<String>) -> Self {
        Self { id: id.into(), kind: PrincipalKind::Admin }
    }
    pub fn system() -> Self {
        Self { id: "system".to_string(), kind: PrincipalKind::System }
    }
}

/// A privileged action subject to authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Proxy,
    AdminRead,
    AdminReload,
    AdminValidate,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Proxy => "proxy",
            Self::AdminRead => "admin.read",
            Self::AdminReload => "admin.reload",
            Self::AdminValidate => "admin.validate",
        }
    }
}

/// The kind of resource an action targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Route,
    AdminEndpoint,
    Config,
}

/// The target of an action (used for auditing and future scoping).
#[derive(Debug, Clone)]
pub struct Resource {
    pub kind: ResourceKind,
    pub name: String,
}

impl Resource {
    pub fn new(kind: ResourceKind, name: impl Into<String>) -> Self {
        Self { kind, name: name.into() }
    }
    pub fn route(name: impl Into<String>) -> Self {
        Self::new(ResourceKind::Route, name)
    }
    pub fn admin_endpoint(name: impl Into<String>) -> Self {
        Self::new(ResourceKind::AdminEndpoint, name)
    }
}

/// An authenticated session that decides what its [`Principal`] may do.
#[derive(Debug, Clone)]
pub struct AuthContext {
    principal: Principal,
}

impl AuthContext {
    pub fn new(principal: Principal) -> Self {
        Self { principal }
    }

    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    /// Authorize `action` on `resource`, or return [`AppError::Unauthorized`].
    /// `resource` is accepted for auditing/scoping even where it is not yet
    /// consulted, so callers always pass through the full boundary.
    pub fn authorize(&self, action: Action, _resource: &Resource) -> Result<(), AppError> {
        let allowed = matches!(
            (self.principal.kind, action),
            (PrincipalKind::System, _)
                | (PrincipalKind::GatewayKey, Action::Proxy)
                | (
                    PrincipalKind::Admin,
                    Action::AdminRead | Action::AdminReload | Action::AdminValidate,
                )
        );
        if allowed {
            Ok(())
        } else {
            Err(AppError::Unauthorized { action: action.as_str().to_string() })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_key_may_proxy_but_not_administer() {
        let ctx = AuthContext::new(Principal::gateway_key("team-alpha"));
        assert!(ctx.authorize(Action::Proxy, &Resource::route("gpt-4o")).is_ok());
        assert!(ctx
            .authorize(Action::AdminReload, &Resource::admin_endpoint("/reload"))
            .is_err());
    }

    #[test]
    fn admin_may_administer_but_not_proxy() {
        let ctx = AuthContext::new(Principal::admin("ops"));
        assert!(ctx
            .authorize(Action::AdminReload, &Resource::admin_endpoint("/reload"))
            .is_ok());
        assert!(ctx.authorize(Action::Proxy, &Resource::route("gpt-4o")).is_err());
    }

    #[test]
    fn unauthorized_error_names_the_action() {
        let ctx = AuthContext::new(Principal::gateway_key("k"));
        let err = ctx
            .authorize(Action::AdminReload, &Resource::admin_endpoint("/reload"))
            .unwrap_err();
        assert_eq!(err.kind(), "pingogate.unauthorized");
        assert_eq!(err.http_status(), 403);
    }
}
