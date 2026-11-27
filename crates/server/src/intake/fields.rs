//! Canonical built-in intake field catalogue.
//!
//! The desktop's legacy wizard still owns its layout during migration, but
//! labels, types and keys come from here so admin configuration and stored
//! values cannot drift.

#[derive(Debug, Clone, Copy)]
pub struct FieldSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub data_type: &'static str,
    pub section: &'static str,
    pub options: &'static [&'static str],
    pub required_by_default: bool,
}

const fn field(
    key: &'static str,
    label: &'static str,
    data_type: &'static str,
    section: &'static str,
) -> FieldSpec {
    FieldSpec {
        key,
        label,
        data_type,
        section,
        options: &[],
        required_by_default: false,
    }
}

const fn select(
    key: &'static str,
    label: &'static str,
    section: &'static str,
    options: &'static [&'static str],
) -> FieldSpec {
    FieldSpec {
        key,
        label,
        data_type: "select",
        section,
        options,
        required_by_default: false,
    }
}

pub const BUILTIN_FIELDS: &[FieldSpec] = &[
    field("first_name", "First name", "text", "Personal details"),
    field("last_name", "Last name", "text", "Personal details"),
    field("family_name", "Family name", "text", "Personal details"),
    field(
        "preferred_name",
        "Preferred name",
        "text",
        "Personal details",
    ),
    field("pronouns", "Pronouns", "text", "Personal details"),
    field("date_of_birth", "Date of birth", "date", "Personal details"),
    select(
        "gender",
        "Gender",
        "Personal details",
        &["female", "male", "non_binary", "prefer_not_to_say"],
    ),
    field("languages", "Languages", "text", "Personal details"),
    field(
        "cultural_identity",
        "Cultural identity",
        "text",
        "Personal details",
    ),
    field("email", "Email", "email", "Contact and address"),
    field("phone", "Phone", "phone", "Contact and address"),
    field(
        "address_line1",
        "Address line 1",
        "text",
        "Contact and address",
    ),
    field(
        "address_line2",
        "Address line 2",
        "text",
        "Contact and address",
    ),
    field("suburb", "Suburb", "text", "Contact and address"),
    select(
        "state",
        "State",
        "Contact and address",
        &["ACT", "NSW", "NT", "QLD", "SA", "TAS", "VIC", "WA"],
    ),
    field("postcode", "Postcode", "text", "Contact and address"),
    field("division", "Division (legacy text)", "text", "Organisation"),
    field("occupation", "Occupation", "text", "Personal details"),
    field("employer", "Employer", "text", "Personal details"),
    field("ndis_number", "NDIS number", "text", "Funding and consent"),
    field(
        "medicare_number",
        "Medicare number",
        "text",
        "Funding and consent",
    ),
    field("funding", "Funding stream", "text", "Funding and consent"),
    field(
        "health_summary",
        "Health summary",
        "long_text",
        "Health and support needs",
    ),
    field(
        "support_needs",
        "Support needs",
        "long_text",
        "Health and support needs",
    ),
    field(
        "risk_summary",
        "Risk summary",
        "long_text",
        "Behaviours and risk",
    ),
    field(
        "behaviour_notes",
        "Behaviour notes",
        "long_text",
        "Behaviours and risk",
    ),
    FieldSpec {
        key: "consent_services",
        label: "Consent to services",
        data_type: "boolean",
        section: "Funding and consent",
        options: &[],
        required_by_default: false,
    },
    FieldSpec {
        key: "consent_share",
        label: "Consent to share information",
        data_type: "boolean",
        section: "Funding and consent",
        options: &[],
        required_by_default: false,
    },
    field(
        "emergency_contact_name",
        "Emergency contact name",
        "text",
        "Support network",
    ),
    field(
        "emergency_contact_phone",
        "Emergency contact phone",
        "phone",
        "Support network",
    ),
    field("gp_name", "GP name", "text", "Support network"),
    field("gp_phone", "GP phone", "phone", "Support network"),
    field("position", "Position", "text", "Employment"),
    select(
        "employment_type",
        "Employment type",
        "Employment",
        &[
            "full_time",
            "part_time",
            "casual",
            "contractor",
            "volunteer",
        ],
    ),
    field("start_date", "Start date", "date", "Employment"),
    field("award", "Award", "text", "Employment"),
    field("classification", "Classification", "text", "Employment"),
    field("pay_point", "Pay point", "text", "Employment"),
    field("superannuation", "Superannuation", "text", "Employment"),
    field(
        "schedule_preferences",
        "Schedule preferences",
        "long_text",
        "Service planning",
    ),
    field("notes", "Notes", "long_text", "Notes"),
    field(
        "organisation_name",
        "Organisation name",
        "text",
        "Organisation",
    ),
];

#[must_use]
pub fn builtin_fields() -> &'static [FieldSpec] {
    BUILTIN_FIELDS
}

#[must_use]
pub fn builtin_field(key: &str) -> Option<&'static FieldSpec> {
    BUILTIN_FIELDS.iter().find(|field| field.key == key)
}

pub fn builtin_field_keys() -> impl Iterator<Item = &'static str> {
    BUILTIN_FIELDS.iter().map(|field| field.key)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn keys_are_unique() {
        let mut seen = HashSet::new();
        for field in BUILTIN_FIELDS {
            assert!(seen.insert(field.key), "duplicate field key {}", field.key);
        }
    }

    #[test]
    fn data_types_are_known() {
        for field in BUILTIN_FIELDS {
            assert!(
                matches!(
                    field.data_type,
                    "text"
                        | "long_text"
                        | "integer"
                        | "decimal"
                        | "boolean"
                        | "date"
                        | "datetime"
                        | "email"
                        | "phone"
                        | "url"
                        | "select"
                        | "multiselect"
                ),
                "unknown type {} for {}",
                field.data_type,
                field.key
            );
        }
    }
}
