//! Worker qualities and client requirements.
//!
//! A quality is anything a worker can hold or be and a client can require:
//! training (PEG feeding), a qualification (first aid) or an attribute
//! (gender, languages). Intrinsic qualities read a `contacts` column; on-demand
//! qualities read `worker_quality_values`. This module owns the shared
//! vocabulary and the predicate evaluation used by rostering.

use chrono::{Duration, NaiveDate};
use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::AppError;

/// Groups used to organise the catalogue in pickers.
pub const CATEGORIES: [(&str, &str); 6] = [
    ("skill", "Skills"),
    ("training", "Training"),
    ("qualification", "Qualifications"),
    ("screening", "Screening and checks"),
    ("attribute", "Worker details"),
    ("preference", "Preferences"),
];

/// Allowed requirement operators. Deliberately tiny.
pub const OPERATORS: [&str; 5] = ["exists", "equals", "not_equals", "in", "contains"];

pub const DATA_TYPES: [&str; 5] = ["boolean", "select", "multiselect", "text", "date"];

pub const STORAGES: [&str; 2] = ["contact_field", "quality_value"];

/// How a worker's value is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    ContactField,
    QualityValue,
}

impl Storage {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "contact_field" => Some(Self::ContactField),
            "quality_value" => Some(Self::QualityValue),
            _ => None,
        }
    }
}

/// One row of the quality catalogue.
#[derive(Debug, Clone, FromRow)]
pub struct Quality {
    pub id: Uuid,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub category: String,
    pub data_type: String,
    pub options: Option<sqlx::types::Json<Value>>,
    pub storage: String,
    pub contact_field: Option<String>,
    pub tracks_expiry: bool,
    pub is_sensitive: bool,
    pub is_system: bool,
    pub is_active: bool,
}

/// A quality a worker holds.
#[derive(Debug, Clone, FromRow)]
pub struct QualityValue {
    pub id: Uuid,
    pub worker_contact_id: Uuid,
    pub quality_id: Uuid,
    pub value: sqlx::types::Json<Value>,
    pub expires_on: Option<NaiveDate>,
    pub evidence: Option<String>,
    pub notes: Option<String>,
}

/// A predicate a client places on workers.
#[derive(Debug, Clone, FromRow)]
pub struct Requirement {
    pub id: Uuid,
    pub client_contact_id: Uuid,
    pub quality_id: Uuid,
    pub operator: String,
    pub value: Option<sqlx::types::Json<Value>>,
    pub severity: String,
    pub reason: Option<String>,
    pub legal_basis: Option<String>,
    pub service_id: Option<Uuid>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

/// The result of matching one requirement against one worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    /// The worker has the quality and it satisfies the predicate.
    Met,
    /// Satisfied, but the quality expires within 30 days.
    Expiring,
    /// The quality is held but has expired.
    Expired,
    /// The value is recorded but does not satisfy the predicate.
    Mismatch,
    /// No value is recorded.
    Unknown,
}

impl RequirementStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Met => "met",
            Self::Expiring => "expiring",
            Self::Expired => "expired",
            Self::Mismatch => "mismatch",
            Self::Unknown => "unknown",
        }
    }

    /// `Expiring` still satisfies the requirement; it is only a warning.
    #[must_use]
    pub const fn is_met(self) -> bool {
        matches!(self, Self::Met | Self::Expiring)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    pub status: RequirementStatus,
    /// Short explanation, e.g. "expires 2027-01-10" or "recorded as male".
    pub detail: Option<String>,
}

pub struct EvaluateInput<'a> {
    pub data_type: &'a str,
    pub operator: &'a str,
    pub required: Option<&'a Value>,
    pub actual: Option<&'a Value>,
    pub tracks_expiry: bool,
    pub expires_on: Option<NaiveDate>,
    pub today: NaiveDate,
}

/// Evaluate one requirement against one worker's value.
#[must_use]
pub fn evaluate(input: EvaluateInput<'_>) -> Evaluation {
    let Some(actual) = input.actual else {
        return Evaluation {
            status: RequirementStatus::Unknown,
            detail: None,
        };
    };

    if input.tracks_expiry {
        if let Some(expires_on) = input.expires_on {
            if expires_on < input.today {
                return Evaluation {
                    status: RequirementStatus::Expired,
                    detail: Some(format!("expired {expires_on}")),
                };
            }
        }
    }

    if !predicate_matches(input.data_type, input.operator, actual, input.required) {
        return Evaluation {
            status: RequirementStatus::Mismatch,
            detail: Some(format!("recorded as {}", describe_value(actual))),
        };
    }

    if input.tracks_expiry {
        if let Some(expires_on) = input.expires_on {
            if expires_on <= input.today + Duration::days(30) {
                return Evaluation {
                    status: RequirementStatus::Expiring,
                    detail: Some(format!("expires {expires_on}")),
                };
            }
        }
    }

    Evaluation {
        status: RequirementStatus::Met,
        detail: input.expires_on.map(|date| format!("expires {date}")),
    }
}

fn predicate_matches(
    data_type: &str,
    operator: &str,
    actual: &Value,
    required: Option<&Value>,
) -> bool {
    match operator {
        "exists" => is_present(actual),
        "equals" => required.is_some_and(|required| actual == required),
        "not_equals" => required.is_some_and(|required| actual != required),
        "in" => {
            let Some(required) = required.and_then(Value::as_array) else {
                return false;
            };
            if data_type == "multiselect" {
                actual.as_array().is_some_and(|values| {
                    values
                        .iter()
                        .any(|value| required.iter().any(|allowed| allowed == value))
                })
            } else {
                required.iter().any(|allowed| allowed == actual)
            }
        }
        "contains" => {
            let Some(required) = required else {
                return false;
            };
            if let Some(values) = actual.as_array() {
                values.iter().any(|value| value == required)
            } else if let (Some(haystack), Some(needle)) = (actual.as_str(), required.as_str()) {
                haystack.to_lowercase().contains(&needle.to_lowercase())
            } else {
                false
            }
        }
        _ => false,
    }
}

/// `false`, empty strings and empty lists do not count as held.
#[must_use]
pub fn is_present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
        Value::Number(_) => true,
    }
}

/// Human-readable value, e.g. `female`, `English, Cantonese`, `held`.
#[must_use]
pub fn describe_value(value: &Value) -> String {
    match value {
        Value::Bool(true) => "held".to_owned(),
        Value::Bool(false) => "not held".to_owned(),
        Value::String(value) => value.replace('_', " "),
        Value::Array(values) => values
            .iter()
            .map(describe_value)
            .collect::<Vec<_>>()
            .join(", "),
        Value::Null => "not recorded".to_owned(),
        other => other.to_string(),
    }
}

/// Turn "PEG feeding" into `peg_feeding`.
#[must_use]
pub fn normalise_key(input: &str) -> Option<String> {
    let key = input
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("_");
    let starts_with_letter = key.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    (starts_with_letter && key.len() <= 63).then_some(key)
}

/// Accept the gender labels used by intake and by the API.
#[must_use]
pub fn normalise_gender(input: &str) -> Option<&'static str> {
    match input
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ' '], "_")
        .as_str()
    {
        "female" => Some("female"),
        "male" => Some("male"),
        "non_binary" => Some("non_binary"),
        "prefer_not_to_say" => Some("prefer_not_to_say"),
        _ => None,
    }
}

/// Trim, drop blanks and de-duplicate languages while preserving order.
#[must_use]
pub fn normalise_languages(values: Vec<String>) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for value in values {
        let value = value.trim().to_owned();
        if !value.is_empty() && !result.iter().any(|seen| seen.eq_ignore_ascii_case(&value)) {
            result.push(value);
        }
    }
    result
}

/// Validate a worker's value against the quality's declared type and options.
pub fn validate_value(quality: &Quality, value: &Value) -> Result<(), AppError> {
    let options = quality
        .options
        .as_ref()
        .and_then(|options| options.0.as_array())
        .cloned()
        .unwrap_or_default();

    match quality.data_type.as_str() {
        "boolean" => {
            if !value.is_boolean() {
                return Err(AppError::validation(format!(
                    "{} needs a true or false value",
                    quality.name
                )));
            }
        }
        "text" => {
            if !value.is_string() {
                return Err(AppError::validation(format!(
                    "{} needs a text value",
                    quality.name
                )));
            }
        }
        "date" => {
            let Some(text) = value.as_str() else {
                return Err(AppError::validation(format!(
                    "{} needs a date (YYYY-MM-DD)",
                    quality.name
                )));
            };
            NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| {
                AppError::validation(format!("{} needs a date (YYYY-MM-DD)", quality.name))
            })?;
        }
        "select" => {
            let Some(text) = value.as_str() else {
                return Err(AppError::validation(format!(
                    "{} needs one of its options",
                    quality.name
                )));
            };
            if !options.is_empty() && !options.iter().any(|option| option.as_str() == Some(text)) {
                return Err(AppError::validation(format!(
                    "{} is not an option for {}",
                    text, quality.name
                )));
            }
        }
        "multiselect" => {
            let Some(values) = value.as_array() else {
                return Err(AppError::validation(format!(
                    "{} needs a list of options",
                    quality.name
                )));
            };
            for value in values {
                let Some(text) = value.as_str() else {
                    return Err(AppError::validation(format!(
                        "{} needs a list of text options",
                        quality.name
                    )));
                };
                if !options.is_empty()
                    && !options.iter().any(|option| option.as_str() == Some(text))
                {
                    return Err(AppError::validation(format!(
                        "{} is not an option for {}",
                        text, quality.name
                    )));
                }
            }
        }
        other => {
            return Err(AppError::validation(format!(
                "unsupported quality data type {other}"
            )));
        }
    }
    Ok(())
}

/// Validate the shape of a requirement predicate.
pub fn validate_requirement(
    quality: &Quality,
    operator: &str,
    value: Option<&Value>,
    severity: &str,
) -> Result<(), AppError> {
    if !OPERATORS.contains(&operator) {
        return Err(AppError::validation(
            "operator must be exists, equals, not_equals, in or contains",
        ));
    }
    if !matches!(severity, "warn" | "block") {
        return Err(AppError::validation("severity must be warn or block"));
    }

    match operator {
        "exists" => {
            if value.is_some() {
                return Err(AppError::validation(
                    "exists requirements do not take a value",
                ));
            }
        }
        "equals" | "not_equals" => {
            let value = value.ok_or_else(|| {
                AppError::validation(format!("{operator} requirements need a value"))
            })?;
            validate_value(quality, value)?;
        }
        "in" => {
            let Some(values) = value.and_then(Value::as_array) else {
                return Err(AppError::validation(
                    "in requirements need a list of allowed values",
                ));
            };
            if values.is_empty() {
                return Err(AppError::validation(
                    "in requirements need at least one value",
                ));
            }
            let options = quality
                .options
                .as_ref()
                .and_then(|options| options.0.as_array())
                .cloned()
                .unwrap_or_default();
            for value in values {
                if matches!(quality.data_type.as_str(), "select" | "multiselect") {
                    let Some(text) = value.as_str() else {
                        return Err(AppError::validation(format!(
                            "{} needs text options",
                            quality.name
                        )));
                    };
                    if !options.is_empty()
                        && !options.iter().any(|option| option.as_str() == Some(text))
                    {
                        return Err(AppError::validation(format!(
                            "{} is not an option for {}",
                            text, quality.name
                        )));
                    }
                } else {
                    validate_value(quality, value)?;
                }
            }
        }
        "contains" => {
            let value =
                value.ok_or_else(|| AppError::validation("contains requirements need a value"))?;
            if quality.data_type == "multiselect" {
                if !value.is_string() {
                    return Err(AppError::validation(
                        "contains needs one value for a list quality",
                    ));
                }
            } else {
                validate_value(quality, value)?;
            }
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn quality(data_type: &str) -> Quality {
        Quality {
            id: Uuid::nil(),
            key: "test".to_owned(),
            name: "Test".to_owned(),
            description: None,
            category: "attribute".to_owned(),
            data_type: data_type.to_owned(),
            options: None,
            storage: "quality_value".to_owned(),
            contact_field: None,
            tracks_expiry: false,
            is_sensitive: false,
            is_system: false,
            is_active: true,
        }
    }

    fn evaluate_value(operator: &str, required: Value, actual: Value) -> RequirementStatus {
        evaluate(EvaluateInput {
            data_type: "text",
            operator,
            required: Some(&required),
            actual: Some(&actual),
            tracks_expiry: false,
            expires_on: None,
            today: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
        })
        .status
    }

    #[test]
    fn equality_and_absence() {
        assert_eq!(
            evaluate_value("equals", json!("female"), json!("female")),
            RequirementStatus::Met
        );
        assert_eq!(
            evaluate_value("equals", json!("female"), json!("male")),
            RequirementStatus::Mismatch
        );

        let result = evaluate(EvaluateInput {
            data_type: "select",
            operator: "equals",
            required: Some(&json!("female")),
            actual: None,
            tracks_expiry: false,
            expires_on: None,
            today: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
        });
        assert_eq!(result.status, RequirementStatus::Unknown);
    }

    #[test]
    fn expiry_wins_over_a_matching_value() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        let result = evaluate(EvaluateInput {
            data_type: "boolean",
            operator: "exists",
            required: None,
            actual: Some(&json!(true)),
            tracks_expiry: true,
            expires_on: Some(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()),
            today,
        });
        assert_eq!(result.status, RequirementStatus::Expired);

        let result = evaluate(EvaluateInput {
            data_type: "boolean",
            operator: "exists",
            required: None,
            actual: Some(&json!(true)),
            tracks_expiry: true,
            expires_on: Some(today + Duration::days(12)),
            today,
        });
        assert_eq!(result.status, RequirementStatus::Expiring);
    }

    #[test]
    fn contains_matches_list_membership() {
        assert_eq!(
            evaluate_value(
                "contains",
                json!("Cantonese"),
                json!(["English", "Cantonese"])
            ),
            RequirementStatus::Met
        );
    }

    #[test]
    fn keys_and_gender_are_normalised() {
        assert_eq!(
            normalise_key("PEG feeding / enteral").as_deref(),
            Some("peg_feeding_enteral")
        );
        assert_eq!(normalise_key("2nd thing"), None);
        assert_eq!(normalise_gender("Non-binary"), Some("non_binary"));
        assert_eq!(
            normalise_gender("Prefer not to say"),
            Some("prefer_not_to_say")
        );
        assert_eq!(normalise_gender("unknown"), None);
    }

    #[test]
    fn boolean_values_are_typed() {
        let quality = quality("boolean");
        assert!(validate_value(&quality, &json!(true)).is_ok());
        assert!(validate_value(&quality, &json!("true")).is_err());
    }
}
