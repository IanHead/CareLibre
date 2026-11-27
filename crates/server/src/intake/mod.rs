//! Intake domain vocabulary shared by the API, admin UI and tests.

pub mod cases;
pub mod fields;

pub use fields::{builtin_field, builtin_field_keys, builtin_fields, FieldSpec};

/// The stage semantics the server branches on. Labels and order are
/// configurable; these kinds are not.
pub const STAGE_KINDS: [&str; 7] = [
    "enquiry",
    "eligibility",
    "risk_assessment",
    "service_planning",
    "custom",
    "onboarded",
    "cancelled",
];

/// The closed action vocabulary for stage policy.
pub const ACTION_KEYS: [&str; 7] = [
    "roster.add",
    "roster.remove",
    "funding.add",
    "funding.edit",
    "funding.remove",
    "profile.edit",
    "relationships.update",
];

/// Subject kinds accepted by intake flows.
pub const SUBJECT_KINDS: [&str; 4] = ["client", "worker", "other_contact", "organisation"];

/// Contact lifecycle values. These are independent of intake stages.
pub const LIFECYCLE_STATUSES: [&str; 7] = [
    "enquiry",
    "onboarding",
    "suspended",
    "active",
    "offboarding",
    "intake_cancelled",
    "offboarded",
];

#[must_use]
pub fn is_stage_kind(value: &str) -> bool {
    STAGE_KINDS.contains(&value)
}

#[must_use]
pub fn is_action_key(value: &str) -> bool {
    ACTION_KEYS.contains(&value)
}

#[must_use]
pub fn is_subject_kind(value: &str) -> bool {
    SUBJECT_KINDS.contains(&value)
}

#[must_use]
pub fn is_lifecycle_status(value: &str) -> bool {
    LIFECYCLE_STATUSES.contains(&value)
}

/// Whether a stage of this kind is terminal by definition.
#[must_use]
pub fn is_terminal_kind(value: &str) -> bool {
    matches!(value, "onboarded" | "cancelled")
}

#[must_use]
pub fn is_terminal_lifecycle(value: &str) -> bool {
    matches!(value, "intake_cancelled" | "offboarded")
}

/// Validate configured field keys against the built-in catalogue plus active
/// custom field keys (`custom:<key>`).
#[must_use]
pub fn field_key_is_valid(key: &str, custom_keys: &[String]) -> bool {
    builtin_field(key).is_some()
        || key
            .strip_prefix("custom:")
            .is_some_and(|custom| custom_keys.iter().any(|known| known == custom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_kinds_and_actions_are_distinct() {
        for kind in STAGE_KINDS {
            assert!(is_stage_kind(kind));
        }
        assert!(!is_stage_kind("nope"));
        for action in ACTION_KEYS {
            assert!(is_action_key(action));
        }
        assert!(!is_action_key("nope"));
    }

    #[test]
    fn terminal_kinds_are_derived_from_the_kind() {
        assert!(is_terminal_kind("onboarded"));
        assert!(is_terminal_kind("cancelled"));
        assert!(!is_terminal_kind("custom"));
        assert!(!is_terminal_kind("enquiry"));
    }

    #[test]
    fn field_keys_include_seeded_defaults() {
        for key in [
            "first_name",
            "family_name",
            "email",
            "phone",
            "organisation_name",
            "funding",
            "schedule_preferences",
            "risk_summary",
            "behaviour_notes",
            "consent_services",
            "consent_share",
        ] {
            assert!(builtin_field(key).is_some(), "missing field {key}");
        }
    }
}
