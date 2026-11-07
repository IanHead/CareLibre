//! Custom field definitions and value validation.
//!
//! Custom fields are **metadata only**: users add rows to
//! `custom_field_definitions`, never columns to tables. Values live in
//! `custom_field_values` as JSONB and are type/rule-checked here and by a
//! database trigger. This keeps user input incapable of breaking the schema.

use std::collections::HashSet;

use serde_json::Value;

/// Supported field types. Keep in sync with the `data_type` check constraint in
/// migration 0007 and the admin form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Text,
    LongText,
    Integer,
    Decimal,
    Boolean,
    Date,
    DateTime,
    Email,
    Phone,
    Url,
    Select,
    MultiSelect,
}

impl FieldType {
    pub const ALL: [Self; 12] = [
        Self::Text,
        Self::LongText,
        Self::Integer,
        Self::Decimal,
        Self::Boolean,
        Self::Date,
        Self::DateTime,
        Self::Email,
        Self::Phone,
        Self::Url,
        Self::Select,
        Self::MultiSelect,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::LongText => "long_text",
            Self::Integer => "integer",
            Self::Decimal => "decimal",
            Self::Boolean => "boolean",
            Self::Date => "date",
            Self::DateTime => "datetime",
            Self::Email => "email",
            Self::Phone => "phone",
            Self::Url => "url",
            Self::Select => "select",
            Self::MultiSelect => "multiselect",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Text => "Short text",
            Self::LongText => "Long text",
            Self::Integer => "Whole number",
            Self::Decimal => "Decimal number",
            Self::Boolean => "Yes / no",
            Self::Date => "Date",
            Self::DateTime => "Date and time",
            Self::Email => "Email address",
            Self::Phone => "Phone number",
            Self::Url => "URL",
            Self::Select => "Single choice",
            Self::MultiSelect => "Multiple choice",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    /// Types that are stored as a JSON string.
    #[must_use]
    pub const fn is_textual(self) -> bool {
        matches!(
            self,
            Self::Text
                | Self::LongText
                | Self::Email
                | Self::Phone
                | Self::Url
                | Self::Date
                | Self::DateTime
                | Self::Select
        )
    }

    #[must_use]
    pub const fn is_numeric(self) -> bool {
        matches!(self, Self::Integer | Self::Decimal)
    }

    #[must_use]
    pub const fn uses_options(self) -> bool {
        matches!(self, Self::Select | Self::MultiSelect)
    }

    #[must_use]
    pub const fn allows_length(self) -> bool {
        matches!(
            self,
            Self::Text | Self::LongText | Self::Email | Self::Phone | Self::Url
        )
    }

    #[must_use]
    pub const fn allows_pattern(self) -> bool {
        matches!(
            self,
            Self::Text | Self::LongText | Self::Email | Self::Phone | Self::Url
        )
    }

    #[must_use]
    pub const fn allows_range(self) -> bool {
        self.is_numeric()
    }
}

/// Per-field validation rules. `None` means "no constraint".
#[derive(Debug, Clone, Default)]
pub struct FieldRules {
    pub required: bool,
    pub min_length: Option<i32>,
    pub max_length: Option<i32>,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub pattern: Option<String>,
    pub options: Option<Vec<String>>,
}

/// A definition being created or edited.
#[derive(Debug, Clone)]
pub struct DefinitionDraft {
    pub entity_type: String,
    pub key: String,
    pub label: String,
    pub data_type: FieldType,
    pub rules: FieldRules,
    pub default_value: Option<Value>,
}

pub const MAX_KEY_LENGTH: usize = 63;
pub const MAX_LABEL_LENGTH: usize = 120;
pub const MAX_OPTIONS: usize = 200;
pub const MAX_OPTION_LENGTH: usize = 120;
pub const MAX_PATTERN_LENGTH: usize = 512;

const RESERVED_KEYS: &[&str] = &[
    "id",
    "organisation_id",
    "tenant_id",
    "entity_type",
    "entity_id",
    "created_at",
    "updated_at",
    "slug",
    "key",
    "data_type",
];

/// Validate a definition before it is written.
///
/// Returns every problem at once so the form can show a complete list.
#[must_use]
pub fn validate_definition(draft: &DefinitionDraft) -> Vec<String> {
    let mut errors = Vec::new();
    let kind = draft.data_type;
    let rules = &draft.rules;

    validate_key(&draft.key, &mut errors);

    let label_length = draft.label.trim().chars().count();
    if label_length == 0 || label_length > MAX_LABEL_LENGTH {
        errors.push(format!(
            "Label must be between 1 and {MAX_LABEL_LENGTH} characters."
        ));
    }

    if rules.min_length.is_some() && !kind.allows_length() {
        errors.push(format!("{} fields do not use length limits.", kind.label()));
    }
    if rules.min_length.is_some_and(|value| value < 0) {
        errors.push("Minimum length cannot be negative.".to_owned());
    }
    if let (Some(min), Some(max)) = (rules.min_length, rules.max_length) {
        if max < min {
            errors.push("Maximum length must be greater than or equal to the minimum.".to_owned());
        }
    }

    if (rules.min_value.is_some() || rules.max_value.is_some()) && !kind.allows_range() {
        errors.push(format!(
            "{} fields do not use numeric ranges.",
            kind.label()
        ));
    }
    if let (Some(min), Some(max)) = (rules.min_value, rules.max_value) {
        if max < min {
            errors.push("Maximum value must be greater than or equal to the minimum.".to_owned());
        }
    }

    if rules.pattern.is_some() && !kind.allows_pattern() {
        errors.push(format!(
            "{} fields do not use a regular expression.",
            kind.label()
        ));
    }
    if let Some(pattern) = &rules.pattern {
        if pattern.len() > MAX_PATTERN_LENGTH {
            errors.push(format!(
                "Regular expressions must be at most {MAX_PATTERN_LENGTH} characters."
            ));
        } else if let Err(err) = regex::Regex::new(pattern) {
            errors.push(format!("Regular expression is invalid: {err}"));
        }
    }

    match (kind.uses_options(), rules.options.as_ref()) {
        (true, None) => errors.push(format!("{} fields need at least one option.", kind.label())),
        (false, Some(_)) => errors.push(format!("{} fields do not use options.", kind.label())),
        (true, Some(options)) => {
            if options.is_empty() {
                errors.push("Add at least one option.".to_owned());
            }
            if options.len() > MAX_OPTIONS {
                errors.push(format!("At most {MAX_OPTIONS} options are allowed."));
            }
            let mut seen = HashSet::new();
            for option in options {
                let trimmed = option.trim();
                if trimmed.is_empty() {
                    errors.push("Options cannot be blank.".to_owned());
                } else if trimmed.chars().count() > MAX_OPTION_LENGTH {
                    errors.push(format!(
                        "Each option must be at most {MAX_OPTION_LENGTH} characters."
                    ));
                }
                if !seen.insert(trimmed.to_ascii_lowercase()) {
                    errors.push(format!("Duplicate option: {trimmed}"));
                }
            }
        }
        (false, None) => {}
    }

    if let Some(default) = &draft.default_value {
        if !default.is_null() {
            if let Err(message) = validate_value(kind, rules, default) {
                errors.push(format!("Default value is invalid: {message}"));
            }
        }
    }

    errors
}

fn validate_key(key: &str, errors: &mut Vec<String>) {
    let trimmed = key.trim();
    let valid_format = (2..=MAX_KEY_LENGTH).contains(&trimmed.len())
        && trimmed
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase())
        && trimmed.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });

    if !valid_format {
        errors.push(format!(
            "Key must be 2-{MAX_KEY_LENGTH} characters, start with a lowercase letter, and contain only a-z, 0-9 and underscore."
        ));
    }

    if RESERVED_KEYS.contains(&trimmed) {
        errors.push(format!(
            "The key `{trimmed}` is reserved for built-in fields."
        ));
    }
}

/// Validate a single stored value against its definition.
pub fn validate_value(kind: FieldType, rules: &FieldRules, value: &Value) -> Result<(), String> {
    if value.is_null() {
        return if rules.required {
            Err("A value is required.".to_owned())
        } else {
            Ok(())
        };
    }

    match kind {
        FieldType::Text | FieldType::LongText | FieldType::Phone => check_text(kind, rules, value),
        FieldType::Email => {
            check_text(kind, rules, value)?;
            let text = value.as_str().unwrap_or_default();
            let valid = match text.split_once('@') {
                Some((local, domain)) => {
                    !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
                }
                None => false,
            };
            if valid {
                Ok(())
            } else {
                Err("Enter a valid email address.".to_owned())
            }
        }
        FieldType::Url => {
            check_text(kind, rules, value)?;
            let text = value.as_str().unwrap_or_default();
            if text.starts_with("http://") || text.starts_with("https://") {
                Ok(())
            } else {
                Err("Enter a URL beginning with http:// or https://.".to_owned())
            }
        }
        FieldType::Date => {
            check_text(kind, rules, value)?;
            let text = value.as_str().unwrap_or_default();
            chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .map(|_| ())
                .map_err(|_| "Enter a valid date (YYYY-MM-DD).".to_owned())
        }
        FieldType::DateTime => {
            check_text(kind, rules, value)?;
            let text = value.as_str().unwrap_or_default();
            chrono::DateTime::parse_from_rfc3339(text)
                .map(|_| ())
                .map_err(|_| "Enter a valid date and time including a timezone.".to_owned())
        }
        FieldType::Integer => {
            let number = value
                .as_i64()
                .ok_or_else(|| "Enter a whole number.".to_owned())?;
            check_numeric_range(rules, number as f64)
        }
        FieldType::Decimal => {
            let number = value.as_f64().ok_or_else(|| "Enter a number.".to_owned())?;
            check_numeric_range(rules, number)
        }
        FieldType::Boolean => value
            .as_bool()
            .map(|_| ())
            .ok_or_else(|| "Choose yes or no.".to_owned()),
        FieldType::Select => {
            let text = value
                .as_str()
                .ok_or_else(|| "Choose one of the options.".to_owned())?;
            if option_exists(rules, text) {
                Ok(())
            } else {
                Err(format!("`{text}` is not one of the configured options."))
            }
        }
        FieldType::MultiSelect => {
            let items = value
                .as_array()
                .ok_or_else(|| "Choose one or more options.".to_owned())?;
            let mut seen = HashSet::new();
            for item in items {
                let text = item
                    .as_str()
                    .ok_or_else(|| "Every selection must be text.".to_owned())?;
                if !option_exists(rules, text) {
                    return Err(format!("`{text}` is not one of the configured options."));
                }
                if !seen.insert(text) {
                    return Err(format!("`{text}` was selected more than once."));
                }
            }
            Ok(())
        }
    }
}

fn check_text(kind: FieldType, rules: &FieldRules, value: &Value) -> Result<(), String> {
    let text = value
        .as_str()
        .ok_or_else(|| format!("{} expects text.", kind.label()))?;

    if kind.allows_length() {
        let length = i32::try_from(text.chars().count()).unwrap_or(i32::MAX);
        if let Some(min) = rules.min_length {
            if length < min {
                return Err(format!("Use at least {min} characters."));
            }
        }
        if let Some(max) = rules.max_length {
            if length > max {
                return Err(format!("Use at most {max} characters."));
            }
        }
    }

    if let Some(pattern) = &rules.pattern {
        let compiled = regex::Regex::new(pattern)
            .map_err(|_| "The validation pattern is invalid.".to_owned())?;
        if !compiled.is_match(text) {
            return Err("Value does not match the required format.".to_owned());
        }
    }

    Ok(())
}

fn check_numeric_range(rules: &FieldRules, number: f64) -> Result<(), String> {
    if let Some(min) = rules.min_value {
        if number < min {
            return Err(format!("Must be at least {min}."));
        }
    }
    if let Some(max) = rules.max_value {
        if number > max {
            return Err(format!("Must be at most {max}."));
        }
    }
    Ok(())
}

fn option_exists(rules: &FieldRules, candidate: &str) -> bool {
    rules
        .options
        .as_ref()
        .is_some_and(|options| options.iter().any(|option| option == candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn draft(kind: FieldType) -> DefinitionDraft {
        DefinitionDraft {
            entity_type: "contact".to_owned(),
            key: "ndis_number".to_owned(),
            label: "NDIS number".to_owned(),
            data_type: kind,
            rules: FieldRules::default(),
            default_value: None,
        }
    }

    #[test]
    fn accepts_a_valid_text_definition() {
        assert!(validate_definition(&draft(FieldType::Text)).is_empty());
    }

    #[test]
    fn rejects_bad_keys() {
        for key in ["", "A", "1abc", "with space", "hyphen-key", "UPPER"] {
            let mut candidate = draft(FieldType::Text);
            candidate.key = key.to_owned();
            assert!(
                validate_definition(&candidate)
                    .iter()
                    .any(|error| error.contains("Key")),
                "{key} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_reserved_keys() {
        let mut candidate = draft(FieldType::Text);
        candidate.key = "created_at".to_owned();
        assert!(validate_definition(&candidate)
            .iter()
            .any(|error| error.contains("reserved")));
    }

    #[test]
    fn select_requires_options_and_rejects_duplicates() {
        let mut candidate = draft(FieldType::Select);
        candidate.rules.options = Some(Vec::new());
        assert!(!validate_definition(&candidate).is_empty());

        candidate.rules.options = Some(vec!["A".to_owned(), "a".to_owned()]);
        assert!(validate_definition(&candidate)
            .iter()
            .any(|error| error.contains("Duplicate")));
    }

    #[test]
    fn rejects_invalid_regex() {
        let mut candidate = draft(FieldType::Text);
        candidate.rules.pattern = Some("(".to_owned());
        assert!(validate_definition(&candidate)
            .iter()
            .any(|error| error.contains("Regular expression")));
    }

    #[test]
    fn length_rules_only_apply_to_text() {
        let mut candidate = draft(FieldType::Integer);
        candidate.rules.min_length = Some(2);
        assert!(!validate_definition(&candidate).is_empty());
    }

    #[test]
    fn validates_text_value_rules() {
        let rules = FieldRules {
            required: true,
            min_length: Some(3),
            max_length: Some(5),
            pattern: Some("^[a-z]+$".to_owned()),
            ..FieldRules::default()
        };

        assert!(validate_value(FieldType::Text, &rules, &json!("abc")).is_ok());
        assert!(validate_value(FieldType::Text, &rules, &json!("ab")).is_err());
        assert!(validate_value(FieldType::Text, &rules, &json!("abcdef")).is_err());
        assert!(validate_value(FieldType::Text, &rules, &json!("ABC")).is_err());
        assert!(validate_value(FieldType::Text, &rules, &Value::Null).is_err());
    }

    #[test]
    fn validates_numeric_ranges() {
        let rules = FieldRules {
            min_value: Some(1.0),
            max_value: Some(10.0),
            ..FieldRules::default()
        };
        assert!(validate_value(FieldType::Integer, &rules, &json!(5)).is_ok());
        assert!(validate_value(FieldType::Integer, &rules, &json!(0)).is_err());
        assert!(validate_value(FieldType::Decimal, &rules, &json!(10.5)).is_err());
        assert!(validate_value(FieldType::Integer, &rules, &json!(1.5)).is_err());
    }

    #[test]
    fn validates_select_and_multiselect() {
        let rules = FieldRules {
            options: Some(vec!["one".to_owned(), "two".to_owned()]),
            ..FieldRules::default()
        };
        assert!(validate_value(FieldType::Select, &rules, &json!("one")).is_ok());
        assert!(validate_value(FieldType::Select, &rules, &json!("three")).is_err());
        assert!(validate_value(FieldType::MultiSelect, &rules, &json!(["one", "two"])).is_ok());
        assert!(validate_value(FieldType::MultiSelect, &rules, &json!(["one", "one"])).is_err());
        assert!(validate_value(FieldType::MultiSelect, &rules, &json!(["three"])).is_err());
    }

    #[test]
    fn validates_special_string_types() {
        let rules = FieldRules::default();
        assert!(validate_value(FieldType::Email, &rules, &json!("a@b.co")).is_ok());
        assert!(validate_value(FieldType::Email, &rules, &json!("nope")).is_err());
        assert!(validate_value(FieldType::Url, &rules, &json!("https://example.com")).is_ok());
        assert!(validate_value(FieldType::Url, &rules, &json!("example.com")).is_err());
        assert!(validate_value(FieldType::Date, &rules, &json!("2026-07-01")).is_ok());
        assert!(validate_value(FieldType::Date, &rules, &json!("01/07/2026")).is_err());
    }
}
