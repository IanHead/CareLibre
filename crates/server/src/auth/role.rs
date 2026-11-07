//! Organisation membership roles.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Full control, including billing/organisation deletion.
    Owner,
    /// Manage users, settings and all care data.
    Admin,
    /// Manage rosters, clients and staff, but not organisation settings.
    Manager,
    /// Day-to-day care delivery.
    Staff,
    /// Read-only access.
    Viewer,
}

impl Role {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Manager => "manager",
            Self::Staff => "staff",
            Self::Viewer => "viewer",
        }
    }

    /// Roles allowed to change organisation-wide settings and membership.
    #[must_use]
    pub const fn can_manage_organisation(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("unknown role: {0}")]
pub struct UnknownRole(pub String);

impl FromStr for Role {
    type Err = UnknownRole;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "manager" => Ok(Self::Manager),
            "staff" => Ok(Self::Staff),
            "viewer" => Ok(Self::Viewer),
            other => Err(UnknownRole(other.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_role() {
        for role in [
            Role::Owner,
            Role::Admin,
            Role::Manager,
            Role::Staff,
            Role::Viewer,
        ] {
            assert_eq!(role.as_str().parse::<Role>().unwrap(), role);
        }
    }

    #[test]
    fn rejects_unknown_roles() {
        assert!("superuser".parse::<Role>().is_err());
    }
}
