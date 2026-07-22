//! Authorization boundary (constitution XX).
//!
//! Every privileged action - proxying a request and every Admin API call -
//! crosses [`AuthContext::authorize`]. There is no global-token equality
//! shortcut: even bootstrap admin credentials are turned into a [`Principal`]
//! and then authorized through this same boundary.

use crate::error::AppError;

/// The category of an authenticated caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalKind {
    /// A client presenting a named gateway key (data path, standalone mode).
    GatewayKey,
    /// A client presenting a virtual key (data path, platform mode). Carries
    /// the virtual-key id as `id` and the owning user as `owner_user_id`
    /// (BYOK visibility, constitution XX).
    VirtualKey,
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
    /// Platform mode only: the user who owns the virtual key (BYOK visibility,
    /// constitution XX). `None` for `GatewayKey` / `Admin` / `System`.
    pub owner_user_id: Option<String>,
}

impl Principal {
    pub fn gateway_key(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: PrincipalKind::GatewayKey,
            owner_user_id: None,
        }
    }
    /// Build a platform-mode virtual-key principal. `id` is the virtual-key
    /// id; `owner_user_id` is the user who owns the virtual key (and the
    /// provider key it references, by BYOK visibility).
    pub fn virtual_key(id: impl Into<String>, owner_user_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: PrincipalKind::VirtualKey,
            owner_user_id: Some(owner_user_id.into()),
        }
    }
    pub fn admin(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: PrincipalKind::Admin,
            owner_user_id: None,
        }
    }
    pub fn system() -> Self {
        Self {
            id: "system".to_string(),
            kind: PrincipalKind::System,
            owner_user_id: None,
        }
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
        Self {
            kind,
            name: name.into(),
        }
    }
    pub fn route(name: impl Into<String>) -> Self {
        Self::new(ResourceKind::Route, name)
    }
    pub fn admin_endpoint(name: impl Into<String>) -> Self {
        Self::new(ResourceKind::AdminEndpoint, name)
    }
    pub fn config() -> Self {
        Self::new(ResourceKind::Config, "config")
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
                | (PrincipalKind::VirtualKey, Action::Proxy)
                | (
                    PrincipalKind::Admin,
                    Action::AdminRead | Action::AdminReload | Action::AdminValidate,
                )
        );
        if allowed {
            Ok(())
        } else {
            Err(AppError::Unauthorized {
                action: action.as_str().to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_key_may_proxy_but_not_administer() {
        let ctx = AuthContext::new(Principal::gateway_key("team-alpha"));
        assert!(ctx
            .authorize(Action::Proxy, &Resource::route("gpt-4o"))
            .is_ok());
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
        assert!(ctx
            .authorize(Action::Proxy, &Resource::route("gpt-4o"))
            .is_err());
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

    #[test]
    fn virtual_key_may_proxy_but_not_administer() {
        let ctx = AuthContext::new(Principal::virtual_key("vk-1", "user-7"));
        assert!(ctx.authorize(Action::Proxy, &Resource::route("gpt-4o")).is_ok());
        assert!(ctx
            .authorize(Action::AdminReload, &Resource::admin_endpoint("/reload"))
            .is_err());
    }

    #[test]
    fn virtual_key_carries_owner_for_byok_visibility() {
        let p = Principal::virtual_key("vk-1", "user-7");
        assert_eq!(p.kind, PrincipalKind::VirtualKey);
        assert_eq!(p.id, "vk-1");
        assert_eq!(p.owner_user_id.as_deref(), Some("user-7"));
    }
}
