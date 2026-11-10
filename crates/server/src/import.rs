//! CSV import: parsing, validation and transactional commit.
//!
//! One file per object. Rows are staged in `import_batches` between review and
//! commit. Commit is all-or-nothing and uses `external_id` for idempotency.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::NaiveDate;
use serde_json::{json, Map, Value};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub const MAX_ROWS: usize = 10_000;
pub const MAX_FILE_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Contact,
    Organisation,
    Division,
    CostCentre,
    Service,
    Rate,
    RateVersion,
    Relationship,
}

impl ObjectType {
    pub const ALL: [Self; 8] = [
        Self::Contact,
        Self::Organisation,
        Self::Division,
        Self::CostCentre,
        Self::Service,
        Self::Rate,
        Self::RateVersion,
        Self::Relationship,
    ];

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Contact => "contacts",
            Self::Organisation => "organisations",
            Self::Division => "divisions",
            Self::CostCentre => "cost_centres",
            Self::Service => "services",
            Self::Rate => "rates",
            Self::RateVersion => "rate_versions",
            Self::Relationship => "relationships",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Contact => "Contacts",
            Self::Organisation => "Organisations",
            Self::Division => "Divisions",
            Self::CostCentre => "Cost centres",
            Self::Service => "Services",
            Self::Rate => "Rates",
            Self::RateVersion => "Rate versions",
            Self::Relationship => "Relationships",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|object| object.key() == value)
    }

    #[must_use]
    pub const fn columns(self) -> &'static [Column] {
        match self {
            Self::Contact => CONTACT_COLUMNS,
            Self::Organisation => ORGANISATION_COLUMNS,
            Self::Division => DIVISION_COLUMNS,
            Self::CostCentre => COST_CENTRE_COLUMNS,
            Self::Service => SERVICE_COLUMNS,
            Self::Rate => RATE_COLUMNS,
            Self::RateVersion => RATE_VERSION_COLUMNS,
            Self::Relationship => RELATIONSHIP_COLUMNS,
        }
    }

    #[must_use]
    pub const fn example(self) -> &'static [&'static str] {
        match self {
            Self::Contact => &[
                "C-0001",
                "Jane Example",
                "Jane",
                "Example",
                "Janie",
                "she/her",
                "1980-04-12",
                "jane.example@example.com",
                "0400 000 000",
                "Example row",
                "true",
            ],
            Self::Organisation => &[
                "O-0001",
                "Example Broker",
                "broker",
                "12345678901",
                "contact@example.com",
                "0400 000 000",
                "https://example.com",
                "Example row",
                "true",
            ],
            Self::Division => &["D-0001", "", "North", "NOTH", "Example row", "true"],
            Self::CostCentre => &[
                "CC-100",
                "",
                "CC-100",
                "Programs",
                "Example row",
                "xero",
                "PROG-100",
                "true",
            ],
            Self::Service => &[
                "S-001",
                "D-0001",
                "PC-01",
                "Personal care",
                "Example row",
                "ndis",
                "01_011_0107_1_1",
                "true",
            ],
            Self::Rate => &[
                "R-001",
                "S-001",
                "Standard weekday",
                "hour",
                "Example row",
                "true",
            ],
            Self::RateVersion => &[
                "RV-001",
                "R-001",
                "98.65",
                "AUD",
                "2026-07-01",
                "2027-07-01",
                "2026-27 schedule",
            ],
            Self::Relationship => &[
                "contact",
                "C-0001",
                "organisation",
                "O-0001",
                "case_manager",
                "client",
                "2026-07-01",
                "",
                "Example row",
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Email,
    Date,
    Decimal,
    Bool,
    Enum(&'static [&'static str]),
    ExternalId,
}

#[derive(Debug)]
pub struct Column {
    pub name: &'static str,
    pub required: bool,
    pub kind: Kind,
    pub max_len: Option<usize>,
    pub note: &'static str,
}

const CONTACT_COLUMNS: &[Column] = &[
    Column {
        name: "external_id",
        required: true,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "Stable key",
    },
    Column {
        name: "display_name",
        required: false,
        kind: Kind::Text,
        max_len: Some(160),
        note: "Calculated from names; ignored",
    },
    Column {
        name: "given_name",
        required: false,
        kind: Kind::Text,
        max_len: Some(160),
        note: "",
    },
    Column {
        name: "family_name",
        required: false,
        kind: Kind::Text,
        max_len: Some(160),
        note: "",
    },
    Column {
        name: "preferred_name",
        required: false,
        kind: Kind::Text,
        max_len: Some(160),
        note: "",
    },
    Column {
        name: "pronouns",
        required: false,
        kind: Kind::Text,
        max_len: Some(60),
        note: "",
    },
    Column {
        name: "date_of_birth",
        required: false,
        kind: Kind::Date,
        max_len: None,
        note: "YYYY-MM-DD",
    },
    Column {
        name: "email",
        required: false,
        kind: Kind::Email,
        max_len: Some(320),
        note: "",
    },
    Column {
        name: "phone",
        required: false,
        kind: Kind::Text,
        max_len: Some(60),
        note: "",
    },
    Column {
        name: "notes",
        required: false,
        kind: Kind::Text,
        max_len: None,
        note: "",
    },
    Column {
        name: "is_active",
        required: false,
        kind: Kind::Bool,
        max_len: None,
        note: "true/false, default true",
    },
];

const ORGANISATION_COLUMNS: &[Column] = &[
    Column {
        name: "external_id",
        required: true,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "Stable key",
    },
    Column {
        name: "name",
        required: true,
        kind: Kind::Text,
        max_len: Some(200),
        note: "",
    },
    Column {
        name: "kind",
        required: false,
        kind: Kind::Text,
        max_len: Some(40),
        note: "provider, broker, health, ...",
    },
    Column {
        name: "abn",
        required: false,
        kind: Kind::Text,
        max_len: Some(11),
        note: "11 digits",
    },
    Column {
        name: "email",
        required: false,
        kind: Kind::Email,
        max_len: Some(320),
        note: "",
    },
    Column {
        name: "phone",
        required: false,
        kind: Kind::Text,
        max_len: Some(60),
        note: "",
    },
    Column {
        name: "website",
        required: false,
        kind: Kind::Text,
        max_len: Some(200),
        note: "",
    },
    Column {
        name: "notes",
        required: false,
        kind: Kind::Text,
        max_len: None,
        note: "",
    },
    Column {
        name: "is_active",
        required: false,
        kind: Kind::Bool,
        max_len: None,
        note: "default true",
    },
];

const DIVISION_COLUMNS: &[Column] = &[
    Column {
        name: "external_id",
        required: true,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "",
    },
    Column {
        name: "parent_external_id",
        required: false,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "Optional parent",
    },
    Column {
        name: "name",
        required: true,
        kind: Kind::Text,
        max_len: Some(160),
        note: "",
    },
    Column {
        name: "code",
        required: false,
        kind: Kind::Text,
        max_len: Some(32),
        note: "Unique per organisation",
    },
    Column {
        name: "description",
        required: false,
        kind: Kind::Text,
        max_len: None,
        note: "",
    },
    Column {
        name: "is_active",
        required: false,
        kind: Kind::Bool,
        max_len: None,
        note: "default true",
    },
];

const COST_CENTRE_COLUMNS: &[Column] = &[
    Column {
        name: "external_id",
        required: true,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "",
    },
    Column {
        name: "parent_external_id",
        required: false,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "Optional parent",
    },
    Column {
        name: "code",
        required: true,
        kind: Kind::Text,
        max_len: Some(32),
        note: "Unique per organisation",
    },
    Column {
        name: "name",
        required: true,
        kind: Kind::Text,
        max_len: Some(160),
        note: "",
    },
    Column {
        name: "description",
        required: false,
        kind: Kind::Text,
        max_len: None,
        note: "",
    },
    Column {
        name: "external_system",
        required: false,
        kind: Kind::Text,
        max_len: Some(40),
        note: "xero, quickbooks, ...",
    },
    Column {
        name: "external_code",
        required: false,
        kind: Kind::Text,
        max_len: Some(64),
        note: "",
    },
    Column {
        name: "is_active",
        required: false,
        kind: Kind::Bool,
        max_len: None,
        note: "default true",
    },
];

const SERVICE_COLUMNS: &[Column] = &[
    Column {
        name: "external_id",
        required: true,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "",
    },
    Column {
        name: "division_external_id",
        required: false,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "Optional division",
    },
    Column {
        name: "code",
        required: true,
        kind: Kind::Text,
        max_len: Some(64),
        note: "Unique per organisation",
    },
    Column {
        name: "name",
        required: true,
        kind: Kind::Text,
        max_len: Some(200),
        note: "",
    },
    Column {
        name: "description",
        required: false,
        kind: Kind::Text,
        max_len: None,
        note: "",
    },
    Column {
        name: "funding_scheme",
        required: true,
        kind: Kind::Enum(&["ndis", "aged_care", "other"]),
        max_len: None,
        note: "ndis | aged_care | other",
    },
    Column {
        name: "funding_code",
        required: false,
        kind: Kind::Text,
        max_len: Some(64),
        note: "",
    },
    Column {
        name: "is_active",
        required: false,
        kind: Kind::Bool,
        max_len: None,
        note: "default true",
    },
];

const RATE_COLUMNS: &[Column] = &[
    Column {
        name: "external_id",
        required: true,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "",
    },
    Column {
        name: "service_external_id",
        required: true,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "Must exist",
    },
    Column {
        name: "name",
        required: true,
        kind: Kind::Text,
        max_len: Some(160),
        note: "Unique per service",
    },
    Column {
        name: "unit_of_measure",
        required: false,
        kind: Kind::Text,
        max_len: Some(32),
        note: "default hour",
    },
    Column {
        name: "description",
        required: false,
        kind: Kind::Text,
        max_len: None,
        note: "",
    },
    Column {
        name: "is_active",
        required: false,
        kind: Kind::Bool,
        max_len: None,
        note: "default true",
    },
];

const RATE_VERSION_COLUMNS: &[Column] = &[
    Column {
        name: "external_id",
        required: true,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "",
    },
    Column {
        name: "rate_external_id",
        required: true,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "Must exist",
    },
    Column {
        name: "amount",
        required: true,
        kind: Kind::Decimal,
        max_len: None,
        note: ">= 0",
    },
    Column {
        name: "currency",
        required: false,
        kind: Kind::Text,
        max_len: Some(3),
        note: "default AUD",
    },
    Column {
        name: "effective_from",
        required: true,
        kind: Kind::Date,
        max_len: None,
        note: "YYYY-MM-DD",
    },
    Column {
        name: "effective_to",
        required: false,
        kind: Kind::Date,
        max_len: None,
        note: "Exclusive; blank = open-ended",
    },
    Column {
        name: "notes",
        required: false,
        kind: Kind::Text,
        max_len: None,
        note: "",
    },
];

const RELATIONSHIP_COLUMNS: &[Column] = &[
    Column {
        name: "from_type",
        required: true,
        kind: Kind::Enum(&["contact", "organisation"]),
        max_len: None,
        note: "contact | organisation",
    },
    Column {
        name: "from_external_id",
        required: true,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "",
    },
    Column {
        name: "to_type",
        required: true,
        kind: Kind::Enum(&["contact", "organisation"]),
        max_len: None,
        note: "contact | organisation",
    },
    Column {
        name: "to_external_id",
        required: true,
        kind: Kind::ExternalId,
        max_len: Some(64),
        note: "",
    },
    Column {
        name: "from_role",
        required: true,
        kind: Kind::Text,
        max_len: Some(64),
        note: "role of the from party, e.g. general_practitioner",
    },
    Column {
        name: "to_role",
        required: true,
        kind: Kind::Text,
        max_len: Some(64),
        note: "role of the to party, e.g. patient",
    },
    Column {
        name: "start_date",
        required: false,
        kind: Kind::Date,
        max_len: None,
        note: "",
    },
    Column {
        name: "end_date",
        required: false,
        kind: Kind::Date,
        max_len: None,
        note: "",
    },
    Column {
        name: "notes",
        required: false,
        kind: Kind::Text,
        max_len: None,
        note: "",
    },
];

pub type Row = BTreeMap<String, String>;

#[derive(Debug, Clone)]
pub struct RowError {
    pub row: usize,
    pub column: Option<String>,
    pub message: String,
    pub value: Option<String>,
}

#[derive(Debug)]
pub struct Parsed {
    pub rows: Vec<Row>,
    pub errors: Vec<RowError>,
}

pub fn parse(object: ObjectType, bytes: &[u8]) -> Result<Parsed, String> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(bytes);

    let headers = reader
        .headers()
        .map_err(|error| format!("could not read the header row: {error}"))?
        .iter()
        .map(|header| header.trim_start_matches('\u{feff}').trim().to_owned())
        .collect::<Vec<_>>();

    let expected = object
        .columns()
        .iter()
        .map(|column| column.name)
        .collect::<HashSet<_>>();

    let mut errors = Vec::new();
    for header in &headers {
        if !expected.contains(header.as_str()) {
            errors.push(RowError {
                row: 0,
                column: Some(header.clone()),
                message: "Unknown column. Download the template for the expected headers."
                    .to_owned(),
                value: None,
            });
        }
    }
    let header_set = headers.iter().cloned().collect::<HashSet<_>>();
    for column in object.columns() {
        if column.required && !header_set.contains(column.name) {
            errors.push(RowError {
                row: 0,
                column: Some(column.name.to_owned()),
                message: "Required column is missing.".to_owned(),
                value: None,
            });
        }
    }
    if !errors.is_empty() {
        return Ok(Parsed {
            rows: Vec::new(),
            errors,
        });
    }

    let mut rows = Vec::new();
    for (index, record) in reader.records().enumerate() {
        let row_number = index + 2;
        if rows.len() >= MAX_ROWS {
            errors.push(RowError {
                row: row_number,
                column: None,
                message: format!("At most {MAX_ROWS} rows are allowed per file."),
                value: None,
            });
            break;
        }
        let record = match record {
            Ok(record) => record,
            Err(error) => {
                errors.push(RowError {
                    row: row_number,
                    column: None,
                    message: format!("Malformed CSV row: {error}"),
                    value: None,
                });
                continue;
            }
        };

        if record.len() > headers.len() {
            errors.push(RowError {
                row: row_number,
                column: None,
                message: format!(
                    "Row has {} columns but the header has {}.",
                    record.len(),
                    headers.len()
                ),
                value: None,
            });
            continue;
        }

        let mut row = Row::new();
        for (position, header) in headers.iter().enumerate() {
            row.insert(
                header.clone(),
                record.get(position).unwrap_or("").trim().to_owned(),
            );
        }
        validate_row(object, row_number, &row, &mut errors);
        rows.push(row);
    }

    if rows.is_empty() && errors.is_empty() {
        errors.push(RowError {
            row: 0,
            column: None,
            message: "The file has no data rows.".to_owned(),
            value: None,
        });
    }

    Ok(Parsed { rows, errors })
}

fn validate_row(object: ObjectType, row_number: usize, row: &Row, errors: &mut Vec<RowError>) {
    for column in object.columns() {
        let value = row.get(column.name).map(String::as_str).unwrap_or("");
        if column.required && value.is_empty() {
            errors.push(RowError {
                row: row_number,
                column: Some(column.name.to_owned()),
                message: "Required value is missing.".to_owned(),
                value: None,
            });
            continue;
        }
        if value.is_empty() {
            continue;
        }
        if let Some(max) = column.max_len {
            if value.chars().count() > max {
                errors.push(RowError {
                    row: row_number,
                    column: Some(column.name.to_owned()),
                    message: format!("Must be at most {max} characters."),
                    value: Some(value.to_owned()),
                });
            }
        }
        match column.kind {
            Kind::Text => {}
            Kind::Email => {
                let valid = match value.split_once('@') {
                    Some((local, domain)) => {
                        !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
                    }
                    None => false,
                };
                if !valid {
                    errors.push(RowError {
                        row: row_number,
                        column: Some(column.name.to_owned()),
                        message: "Invalid email address.".to_owned(),
                        value: Some(value.to_owned()),
                    });
                }
            }
            Kind::Date => {
                if NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err() {
                    errors.push(RowError {
                        row: row_number,
                        column: Some(column.name.to_owned()),
                        message: "Expected YYYY-MM-DD.".to_owned(),
                        value: Some(value.to_owned()),
                    });
                }
            }
            Kind::Decimal => {
                if value.parse::<f64>().is_err() {
                    errors.push(RowError {
                        row: row_number,
                        column: Some(column.name.to_owned()),
                        message: "Expected a number.".to_owned(),
                        value: Some(value.to_owned()),
                    });
                }
            }
            Kind::Bool => {
                if value != "true" && value != "false" {
                    errors.push(RowError {
                        row: row_number,
                        column: Some(column.name.to_owned()),
                        message: "Expected true or false.".to_owned(),
                        value: Some(value.to_owned()),
                    });
                }
            }
            Kind::Enum(options) => {
                if !options.contains(&value) {
                    errors.push(RowError {
                        row: row_number,
                        column: Some(column.name.to_owned()),
                        message: format!("Expected one of: {}.", options.join(", ")),
                        value: Some(value.to_owned()),
                    });
                }
            }
            Kind::ExternalId => {
                let valid = (1..=64).contains(&value.len())
                    && value.chars().all(|character| {
                        character.is_ascii_alphanumeric() || "._-".contains(character)
                    });
                if !valid {
                    errors.push(RowError {
                        row: row_number,
                        column: Some(column.name.to_owned()),
                        message: "Use 1-64 characters: A-Z a-z 0-9 . _ -".to_owned(),
                        value: Some(value.to_owned()),
                    });
                }
            }
        }
    }

    match object {
        ObjectType::Contact => {
            let given = row
                .get("given_name")
                .map(String::as_str)
                .unwrap_or("")
                .trim();
            let family = row
                .get("family_name")
                .map(String::as_str)
                .unwrap_or("")
                .trim();
            if given.is_empty() && family.is_empty() {
                errors.push(RowError {
                    row: row_number,
                    column: Some("given_name".to_owned()),
                    message: "Provide given_name or family_name; display_name is calculated."
                        .to_owned(),
                    value: None,
                });
            }
        }
        ObjectType::Organisation => {
            if let Some(abn) = row.get("abn").filter(|value| !value.is_empty()) {
                if abn.len() != 11 || !abn.chars().all(|character| character.is_ascii_digit()) {
                    errors.push(RowError {
                        row: row_number,
                        column: Some("abn".to_owned()),
                        message: "ABN must be exactly 11 digits.".to_owned(),
                        value: Some(abn.clone()),
                    });
                }
            }
        }
        ObjectType::RateVersion => {
            if let (Some(from), Some(to)) = (
                row.get("effective_from").filter(|value| !value.is_empty()),
                row.get("effective_to").filter(|value| !value.is_empty()),
            ) {
                if let (Ok(from), Ok(to)) = (
                    NaiveDate::parse_from_str(from, "%Y-%m-%d"),
                    NaiveDate::parse_from_str(to, "%Y-%m-%d"),
                ) {
                    if to <= from {
                        errors.push(RowError {
                            row: row_number,
                            column: Some("effective_to".to_owned()),
                            message: "effective_to must be after effective_from.".to_owned(),
                            value: Some(to.to_string()),
                        });
                    }
                }
            }
        }
        ObjectType::Relationship => {
            if let (Some(start), Some(end)) = (
                row.get("start_date").filter(|value| !value.is_empty()),
                row.get("end_date").filter(|value| !value.is_empty()),
            ) {
                if let (Ok(start), Ok(end)) = (
                    NaiveDate::parse_from_str(start, "%Y-%m-%d"),
                    NaiveDate::parse_from_str(end, "%Y-%m-%d"),
                ) {
                    if end < start {
                        errors.push(RowError {
                            row: row_number,
                            column: Some("end_date".to_owned()),
                            message: "end_date cannot be before start_date.".to_owned(),
                            value: Some(end.to_string()),
                        });
                    }
                }
            }
            if row.get("from_type") == row.get("to_type")
                && row.get("from_external_id") == row.get("to_external_id")
            {
                errors.push(RowError {
                    row: row_number,
                    column: None,
                    message: "A relationship cannot point at itself.".to_owned(),
                    value: None,
                });
            }
        }
        _ => {}
    }
}

/// Check file-internal duplicates and database conflicts/references.
pub async fn prevalidate(
    pool: &PgPool,
    organisation_id: Uuid,
    object: ObjectType,
    rows: &[Row],
    create_only: bool,
) -> Result<Vec<RowError>, sqlx::Error> {
    let mut errors = Vec::new();

    if object != ObjectType::Relationship {
        let mut seen = HashMap::new();
        for (index, row) in rows.iter().enumerate() {
            let external_id = row.get("external_id").cloned().unwrap_or_default();
            if let Some(previous) = seen.insert(external_id.clone(), index + 2) {
                errors.push(RowError {
                    row: index + 2,
                    column: Some("external_id".to_owned()),
                    message: format!("Duplicate external_id in this file (also row {previous})."),
                    value: Some(external_id),
                });
            }
        }

        if create_only {
            let existing = existing_external_ids(pool, organisation_id, object).await?;
            for (index, row) in rows.iter().enumerate() {
                if let Some(external_id) = row.get("external_id") {
                    if existing.contains(external_id) {
                        errors.push(RowError {
                            row: index + 2,
                            column: Some("external_id".to_owned()),
                            message: "This external_id already exists. Use update_by_external_id to change it.".to_owned(),
                            value: Some(external_id.clone()),
                        });
                    }
                }
            }
        }
    }

    let references = match object {
        ObjectType::Division | ObjectType::CostCentre => vec!["parent_external_id"],
        ObjectType::Service => vec!["division_external_id"],
        ObjectType::Rate => vec!["service_external_id"],
        ObjectType::RateVersion => vec!["rate_external_id"],
        _ => Vec::new(),
    };

    if !references.is_empty() {
        let mut file_ids = rows
            .iter()
            .filter_map(|row| row.get("external_id").cloned())
            .collect::<HashSet<_>>();
        for (index, row) in rows.iter().enumerate() {
            for reference in &references {
                if let Some(value) = row.get(*reference).filter(|value| !value.is_empty()) {
                    let target = match *reference {
                        "parent_external_id" => object,
                        "division_external_id" => ObjectType::Division,
                        "service_external_id" => ObjectType::Service,
                        "rate_external_id" => ObjectType::Rate,
                        _ => object,
                    };
                    if *reference == "parent_external_id" && file_ids.contains(value) {
                        continue;
                    }
                    let exists = existing_external_ids(pool, organisation_id, target)
                        .await?
                        .contains(value);
                    if !exists && !file_ids.contains(value) {
                        errors.push(RowError {
                            row: index + 2,
                            column: Some((*reference).to_owned()),
                            message: format!("`{value}` does not exist yet."),
                            value: Some(value.clone()),
                        });
                    }
                }
            }
        }
        file_ids.clear();
    }

    if object == ObjectType::Relationship {
        let contacts = existing_external_ids(pool, organisation_id, ObjectType::Contact).await?;
        let organisations =
            existing_external_ids(pool, organisation_id, ObjectType::Organisation).await?;
        for (index, row) in rows.iter().enumerate() {
            for (side, kind) in [("from", "from_type"), ("to", "to_type")] {
                let entity_type = row.get(kind).map(String::as_str).unwrap_or("");
                let external_id = row
                    .get(&format!("{side}_external_id"))
                    .map(String::as_str)
                    .unwrap_or("");
                let exists = match entity_type {
                    "contact" => contacts.contains(external_id),
                    "organisation" => organisations.contains(external_id),
                    _ => false,
                };
                if !exists {
                    errors.push(RowError {
                        row: index + 2,
                        column: Some(format!("{side}_external_id")),
                        message: format!("{entity_type} `{external_id}` was not found."),
                        value: Some(external_id.to_owned()),
                    });
                }
            }
        }
    }

    Ok(errors)
}

async fn existing_external_ids(
    pool: &PgPool,
    organisation_id: Uuid,
    object: ObjectType,
) -> Result<HashSet<String>, sqlx::Error> {
    let sql = match object {
        ObjectType::Contact => {
            "SELECT external_id FROM contacts WHERE organisation_id = $1 AND external_id IS NOT NULL"
        }
        ObjectType::Organisation => {
            "SELECT external_id FROM organisations WHERE tenant_id = $1 AND external_id IS NOT NULL"
        }
        ObjectType::Division => {
            "SELECT external_id FROM divisions WHERE organisation_id = $1 AND external_id IS NOT NULL"
        }
        ObjectType::CostCentre => {
            "SELECT external_id FROM cost_centres WHERE organisation_id = $1 AND external_id IS NOT NULL"
        }
        ObjectType::Service => {
            "SELECT external_id FROM services WHERE organisation_id = $1 AND external_id IS NOT NULL"
        }
        ObjectType::Rate => {
            "SELECT external_id FROM rates WHERE organisation_id = $1 AND external_id IS NOT NULL"
        }
        ObjectType::RateVersion => {
            "SELECT external_id FROM rate_versions WHERE organisation_id = $1 AND external_id IS NOT NULL"
        }
        ObjectType::Relationship => "SELECT '' WHERE false",
    };

    let values: Vec<String> = sqlx::query_scalar(sql)
        .bind(organisation_id)
        .fetch_all(pool)
        .await?;
    Ok(values.into_iter().collect())
}

pub fn template_csv(object: ObjectType) -> String {
    let headers = object
        .columns()
        .iter()
        .map(|column| column.name)
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    {
        let mut writer = csv::Writer::from_writer(&mut output);
        let _ = writer.write_record(&headers);
        let _ = writer.write_record(object.example());
        let _ = writer.flush();
    }
    String::from_utf8(output).unwrap_or_default()
}

pub fn store_rows(rows: &[Row]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                let mut map = Map::new();
                for (key, value) in row {
                    map.insert(key.clone(), json!(value));
                }
                Value::Object(map)
            })
            .collect(),
    )
}

pub fn load_rows(value: &Value) -> Vec<Row> {
    value
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let object = row.as_object()?;
                    let mut map = Row::new();
                    for (key, value) in object {
                        map.insert(key.clone(), value.as_str().unwrap_or("").to_owned());
                    }
                    Some(map)
                })
                .collect()
        })
        .unwrap_or_default()
}

pub async fn commit(
    pool: &PgPool,
    organisation_id: Uuid,
    object: ObjectType,
    rows: &[Row],
    create_only: bool,
) -> Result<(i64, i64), String> {
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    let mut counts = (0, 0);

    match object {
        ObjectType::Contact => {
            for (index, row) in rows.iter().enumerate() {
                let updated = upsert_contact(&mut tx, organisation_id, row, create_only)
                    .await
                    .map_err(|error| format!("row {}: {error}", index + 2))?;
                if updated {
                    counts.1 += 1;
                } else {
                    counts.0 += 1;
                }
            }
        }
        ObjectType::Organisation => {
            for (index, row) in rows.iter().enumerate() {
                let updated = upsert_organisation(&mut tx, organisation_id, row, create_only)
                    .await
                    .map_err(|error| format!("row {}: {error}", index + 2))?;
                if updated {
                    counts.1 += 1;
                } else {
                    counts.0 += 1;
                }
            }
        }
        ObjectType::Division => {
            counts = commit_hierarchy(&mut tx, organisation_id, object, rows, create_only).await?;
        }
        ObjectType::CostCentre => {
            counts = commit_hierarchy(&mut tx, organisation_id, object, rows, create_only).await?;
        }
        ObjectType::Service => {
            for (index, row) in rows.iter().enumerate() {
                let updated = upsert_service(&mut tx, organisation_id, row, create_only)
                    .await
                    .map_err(|error| format!("row {}: {error}", index + 2))?;
                if updated {
                    counts.1 += 1;
                } else {
                    counts.0 += 1;
                }
            }
        }
        ObjectType::Rate => {
            for (index, row) in rows.iter().enumerate() {
                let updated = upsert_rate(&mut tx, organisation_id, row, create_only)
                    .await
                    .map_err(|error| format!("row {}: {error}", index + 2))?;
                if updated {
                    counts.1 += 1;
                } else {
                    counts.0 += 1;
                }
            }
        }
        ObjectType::RateVersion => {
            for (index, row) in rows.iter().enumerate() {
                let updated = upsert_rate_version(&mut tx, organisation_id, row, create_only)
                    .await
                    .map_err(|error| format!("row {}: {error}", index + 2))?;
                if updated {
                    counts.1 += 1;
                } else {
                    counts.0 += 1;
                }
            }
        }
        ObjectType::Relationship => {
            for (index, row) in rows.iter().enumerate() {
                insert_relationship(&mut tx, organisation_id, row)
                    .await
                    .map_err(|error| format!("row {}: {error}", index + 2))?;
                counts.0 += 1;
            }
        }
    }

    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(counts)
}

// ---------------------------------------------------------------------------
// Per-object writes
// ---------------------------------------------------------------------------

fn value<'a>(row: &'a Row, key: &str) -> &'a str {
    row.get(key).map(String::as_str).unwrap_or("")
}

fn optional<'a>(row: &'a Row, key: &str) -> Option<&'a str> {
    row.get(key)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

fn boolean(row: &Row, key: &str) -> bool {
    !matches!(optional(row, key), Some("false"))
}

fn date(row: &Row, key: &str) -> Option<NaiveDate> {
    optional(row, key).and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
}

fn contact_display_name(row: &Row) -> String {
    let preferred = value(row, "preferred_name").trim();
    let given = value(row, "given_name").trim();
    let family = value(row, "family_name").trim();
    let first = if preferred.is_empty() {
        given
    } else {
        preferred
    };
    match (first.is_empty(), family.is_empty()) {
        (false, false) => format!("{first} {family}"),
        (false, true) => first.to_owned(),
        (true, false) => family.to_owned(),
        (true, true) => String::new(),
    }
}

async fn upsert_contact(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    row: &Row,
    create_only: bool,
) -> Result<bool, String> {
    let external_id = value(row, "external_id");
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM contacts WHERE organisation_id = $1 AND external_id = $2",
    )
    .bind(organisation)
    .bind(external_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| error.to_string())?;

    if let Some(id) = existing {
        if create_only {
            return Err("external_id already exists".to_owned());
        }
        sqlx::query(
            "UPDATE contacts SET display_name=$3, given_name=$4, family_name=$5,
                    preferred_name=$6, pronouns=$7, date_of_birth=$8, email=$9,
                    phone=$10, notes=$11, is_active=$12
             WHERE id=$1 AND organisation_id=$2",
        )
        .bind(id)
        .bind(organisation)
        .bind(contact_display_name(row))
        .bind(optional(row, "given_name"))
        .bind(optional(row, "family_name"))
        .bind(optional(row, "preferred_name"))
        .bind(optional(row, "pronouns"))
        .bind(date(row, "date_of_birth"))
        .bind(optional(row, "email"))
        .bind(optional(row, "phone"))
        .bind(optional(row, "notes"))
        .bind(boolean(row, "is_active"))
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
        Ok(true)
    } else {
        sqlx::query(
            "INSERT INTO contacts
                (id, organisation_id, external_id, display_name, given_name, family_name,
                 preferred_name, pronouns, date_of_birth, email, phone, notes, is_active)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
        )
        .bind(Uuid::now_v7())
        .bind(organisation)
        .bind(external_id)
        .bind(contact_display_name(row))
        .bind(optional(row, "given_name"))
        .bind(optional(row, "family_name"))
        .bind(optional(row, "preferred_name"))
        .bind(optional(row, "pronouns"))
        .bind(date(row, "date_of_birth"))
        .bind(optional(row, "email"))
        .bind(optional(row, "phone"))
        .bind(optional(row, "notes"))
        .bind(boolean(row, "is_active"))
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
        Ok(false)
    }
}

async fn upsert_organisation(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    row: &Row,
    create_only: bool,
) -> Result<bool, String> {
    let external_id = value(row, "external_id");
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM organisations WHERE tenant_id = $1 AND external_id = $2",
    )
    .bind(organisation)
    .bind(external_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| error.to_string())?;

    if let Some(id) = existing {
        if create_only {
            return Err("external_id already exists".to_owned());
        }
        sqlx::query(
            "UPDATE organisations SET name=$3, kind=$4, abn=$5, email=$6,
                    phone=$7, website=$8, notes=$9, is_active=$10
             WHERE id=$1 AND tenant_id=$2",
        )
        .bind(id)
        .bind(organisation)
        .bind(value(row, "name"))
        .bind(optional(row, "kind"))
        .bind(optional(row, "abn"))
        .bind(optional(row, "email"))
        .bind(optional(row, "phone"))
        .bind(optional(row, "website"))
        .bind(optional(row, "notes"))
        .bind(boolean(row, "is_active"))
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
        Ok(true)
    } else {
        sqlx::query(
            "INSERT INTO organisations
                (id, tenant_id, external_id, name, kind, abn, email, phone, website,
                 notes, is_active)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        )
        .bind(Uuid::now_v7())
        .bind(organisation)
        .bind(external_id)
        .bind(value(row, "name"))
        .bind(optional(row, "kind"))
        .bind(optional(row, "abn"))
        .bind(optional(row, "email"))
        .bind(optional(row, "phone"))
        .bind(optional(row, "website"))
        .bind(optional(row, "notes"))
        .bind(boolean(row, "is_active"))
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
        Ok(false)
    }
}

async fn commit_hierarchy(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    object: ObjectType,
    rows: &[Row],
    create_only: bool,
) -> Result<(i64, i64), String> {
    let mut counts = (0, 0);
    let mut pending_parents: Vec<(String, String)> = Vec::new();

    for (index, row) in rows.iter().enumerate() {
        let external_id = value(row, "external_id");
        let parent = optional(row, "parent_external_id");

        let existing: Option<Uuid> = match object {
            ObjectType::Division => sqlx::query_scalar(
                "SELECT id FROM divisions WHERE organisation_id=$1 AND external_id=$2",
            )
            .bind(organisation)
            .bind(external_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| format!("row {}: {error}", index + 2))?,
            _ => sqlx::query_scalar(
                "SELECT id FROM cost_centres WHERE organisation_id=$1 AND external_id=$2",
            )
            .bind(organisation)
            .bind(external_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| format!("row {}: {error}", index + 2))?,
        };

        if let Some(id) = existing {
            if create_only {
                return Err(format!("row {}: external_id already exists", index + 2));
            }
            if object == ObjectType::Division {
                sqlx::query(
                    "UPDATE divisions SET name=$3, code=$4, description=$5, is_active=$6
                     WHERE id=$1 AND organisation_id=$2",
                )
                .bind(id)
                .bind(organisation)
                .bind(value(row, "name"))
                .bind(optional(row, "code"))
                .bind(optional(row, "description"))
                .bind(boolean(row, "is_active"))
                .execute(&mut **tx)
                .await
                .map_err(|error| format!("row {}: {error}", index + 2))?;
            } else {
                sqlx::query(
                    "UPDATE cost_centres SET code=$3, name=$4, description=$5,
                            external_system=$6, external_code=$7, is_active=$8
                     WHERE id=$1 AND organisation_id=$2",
                )
                .bind(id)
                .bind(organisation)
                .bind(value(row, "code"))
                .bind(value(row, "name"))
                .bind(optional(row, "description"))
                .bind(optional(row, "external_system"))
                .bind(optional(row, "external_code"))
                .bind(boolean(row, "is_active"))
                .execute(&mut **tx)
                .await
                .map_err(|error| format!("row {}: {error}", index + 2))?;
            }
            counts.1 += 1;
        } else if object == ObjectType::Division {
            sqlx::query(
                "INSERT INTO divisions
                    (id, organisation_id, external_id, name, code, description, is_active)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)",
            )
            .bind(Uuid::now_v7())
            .bind(organisation)
            .bind(external_id)
            .bind(value(row, "name"))
            .bind(optional(row, "code"))
            .bind(optional(row, "description"))
            .bind(boolean(row, "is_active"))
            .execute(&mut **tx)
            .await
            .map_err(|error| format!("row {}: {error}", index + 2))?;
            counts.0 += 1;
        } else {
            sqlx::query(
                "INSERT INTO cost_centres
                    (id, organisation_id, external_id, code, name, description,
                     external_system, external_code, is_active)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            )
            .bind(Uuid::now_v7())
            .bind(organisation)
            .bind(external_id)
            .bind(value(row, "code"))
            .bind(value(row, "name"))
            .bind(optional(row, "description"))
            .bind(optional(row, "external_system"))
            .bind(optional(row, "external_code"))
            .bind(boolean(row, "is_active"))
            .execute(&mut **tx)
            .await
            .map_err(|error| format!("row {}: {error}", index + 2))?;
            counts.0 += 1;
        }

        if let Some(parent) = parent {
            pending_parents.push((external_id.to_owned(), parent.to_owned()));
        }
    }

    for (external_id, parent_external_id) in pending_parents {
        if external_id == parent_external_id {
            return Err(format!("`{external_id}` cannot be its own parent"));
        }
        let parent_id: Option<Uuid> = match object {
            ObjectType::Division => sqlx::query_scalar(
                "SELECT id FROM divisions WHERE organisation_id=$1 AND external_id=$2",
            )
            .bind(organisation)
            .bind(&parent_external_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| error.to_string())?,
            _ => sqlx::query_scalar(
                "SELECT id FROM cost_centres WHERE organisation_id=$1 AND external_id=$2",
            )
            .bind(organisation)
            .bind(&parent_external_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| error.to_string())?,
        };
        let Some(parent_id) = parent_id else {
            return Err(format!(
                "parent `{parent_external_id}` for `{external_id}` was not found"
            ));
        };
        let sql = if object == ObjectType::Division {
            "UPDATE divisions SET parent_id=$3 WHERE organisation_id=$1 AND external_id=$2"
        } else {
            "UPDATE cost_centres SET parent_id=$3 WHERE organisation_id=$1 AND external_id=$2"
        };
        sqlx::query(sql)
            .bind(organisation)
            .bind(&external_id)
            .bind(parent_id)
            .execute(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;
    }

    Ok(counts)
}

async fn upsert_service(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    row: &Row,
    create_only: bool,
) -> Result<bool, String> {
    let external_id = value(row, "external_id");
    let existing: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM services WHERE organisation_id=$1 AND external_id=$2")
            .bind(organisation)
            .bind(external_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;

    let division_id: Option<Uuid> = match optional(row, "division_external_id") {
        Some(code) => Some(
            sqlx::query_scalar(
                "SELECT id FROM divisions WHERE organisation_id=$1 AND external_id=$2",
            )
            .bind(organisation)
            .bind(code)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("division `{code}` was not found"))?,
        ),
        None => None,
    };

    if let Some(id) = existing {
        if create_only {
            return Err("external_id already exists".to_owned());
        }
        sqlx::query(
            "UPDATE services SET division_id=$3, code=$4, name=$5, description=$6,
                    funding_scheme=$7, funding_code=$8, is_active=$9
             WHERE id=$1 AND organisation_id=$2",
        )
        .bind(id)
        .bind(organisation)
        .bind(division_id)
        .bind(value(row, "code"))
        .bind(value(row, "name"))
        .bind(optional(row, "description"))
        .bind(value(row, "funding_scheme"))
        .bind(optional(row, "funding_code"))
        .bind(boolean(row, "is_active"))
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
        Ok(true)
    } else {
        sqlx::query(
            "INSERT INTO services
                (id, organisation_id, external_id, division_id, code, name, description,
                 funding_scheme, funding_code, is_active)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(Uuid::now_v7())
        .bind(organisation)
        .bind(external_id)
        .bind(division_id)
        .bind(value(row, "code"))
        .bind(value(row, "name"))
        .bind(optional(row, "description"))
        .bind(value(row, "funding_scheme"))
        .bind(optional(row, "funding_code"))
        .bind(boolean(row, "is_active"))
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
        Ok(false)
    }
}

async fn upsert_rate(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    row: &Row,
    create_only: bool,
) -> Result<bool, String> {
    let external_id = value(row, "external_id");
    let service_code = value(row, "service_external_id");
    let service_id: Uuid =
        sqlx::query_scalar("SELECT id FROM services WHERE organisation_id=$1 AND external_id=$2")
            .bind(organisation)
            .bind(service_code)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("service `{service_code}` was not found"))?;

    let existing: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM rates WHERE organisation_id=$1 AND external_id=$2")
            .bind(organisation)
            .bind(external_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;

    let unit = optional(row, "unit_of_measure").unwrap_or("hour");

    if let Some(id) = existing {
        if create_only {
            return Err("external_id already exists".to_owned());
        }
        sqlx::query(
            "UPDATE rates SET service_id=$3, name=$4, unit_of_measure=$5,
                    description=$6, is_active=$7
             WHERE id=$1 AND organisation_id=$2",
        )
        .bind(id)
        .bind(organisation)
        .bind(service_id)
        .bind(value(row, "name"))
        .bind(unit)
        .bind(optional(row, "description"))
        .bind(boolean(row, "is_active"))
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
        Ok(true)
    } else {
        sqlx::query(
            "INSERT INTO rates
                (id, organisation_id, external_id, service_id, name, unit_of_measure,
                 description, is_active)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(Uuid::now_v7())
        .bind(organisation)
        .bind(external_id)
        .bind(service_id)
        .bind(value(row, "name"))
        .bind(unit)
        .bind(optional(row, "description"))
        .bind(boolean(row, "is_active"))
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
        Ok(false)
    }
}

async fn upsert_rate_version(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    row: &Row,
    create_only: bool,
) -> Result<bool, String> {
    let external_id = value(row, "external_id");
    let rate_code = value(row, "rate_external_id");
    let rate_id: Uuid =
        sqlx::query_scalar("SELECT id FROM rates WHERE organisation_id=$1 AND external_id=$2")
            .bind(organisation)
            .bind(rate_code)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("rate `{rate_code}` was not found"))?;

    let amount: f64 = value(row, "amount")
        .parse()
        .map_err(|_| "amount must be a number".to_owned())?;
    let from =
        date(row, "effective_from").ok_or_else(|| "effective_from is required".to_owned())?;
    let to = date(row, "effective_to");
    let currency = optional(row, "currency").unwrap_or("AUD");

    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM rate_versions WHERE organisation_id=$1 AND external_id=$2",
    )
    .bind(organisation)
    .bind(external_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| error.to_string())?;

    if let Some(id) = existing {
        if create_only {
            return Err("external_id already exists".to_owned());
        }
        sqlx::query(
            "UPDATE rate_versions SET rate_id=$3, amount=$4, currency=$5,
                    effective_from=$6, effective_to=$7, notes=$8
             WHERE id=$1 AND organisation_id=$2",
        )
        .bind(id)
        .bind(organisation)
        .bind(rate_id)
        .bind(amount)
        .bind(currency)
        .bind(from)
        .bind(to)
        .bind(optional(row, "notes"))
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
        return Ok(true);
    } else {
        sqlx::query(
            "INSERT INTO rate_versions
                (id, organisation_id, external_id, rate_id, amount, currency,
                 effective_from, effective_to, notes)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(Uuid::now_v7())
        .bind(organisation)
        .bind(external_id)
        .bind(rate_id)
        .bind(amount)
        .bind(currency)
        .bind(from)
        .bind(to)
        .bind(optional(row, "notes"))
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
    }

    Ok(false)
}

async fn insert_relationship(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    row: &Row,
) -> Result<(), String> {
    let from_type = value(row, "from_type");
    let to_type = value(row, "to_type");
    let from_external = value(row, "from_external_id");
    let to_external = value(row, "to_external_id");

    let from_contact = if from_type == "contact" {
        lookup_contact(tx, organisation, from_external).await?
    } else {
        None
    };
    let from_org = if from_type == "organisation" {
        lookup_organisation(tx, organisation, from_external).await?
    } else {
        None
    };
    let to_contact = if to_type == "contact" {
        lookup_contact(tx, organisation, to_external).await?
    } else {
        None
    };
    let to_org = if to_type == "organisation" {
        lookup_organisation(tx, organisation, to_external).await?
    } else {
        None
    };

    let from_role = value(row, "from_role");
    let to_role = value(row, "to_role");
    crate::relationship_types::ensure_allowed(&mut **tx, organisation, from_role, to_role)
        .await
        .map_err(|error| error.to_string())?;
    let duplicate: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM relationships
            WHERE organisation_id=$1
              AND from_contact_id IS NOT DISTINCT FROM $2
              AND from_organisation_id IS NOT DISTINCT FROM $3
              AND to_contact_id IS NOT DISTINCT FROM $4
              AND to_organisation_id IS NOT DISTINCT FROM $5
              AND from_role=$6
              AND to_role=$7
              AND removed_at IS NULL
              AND (end_date IS NULL OR end_date >= CURRENT_DATE)
        )",
    )
    .bind(organisation)
    .bind(from_contact)
    .bind(from_org)
    .bind(to_contact)
    .bind(to_org)
    .bind(from_role)
    .bind(to_role)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| error.to_string())?;

    if duplicate {
        return Err("duplicate relationship".to_owned());
    }

    sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_contact_id, from_organisation_id,
             to_contact_id, to_organisation_id, from_role, to_role,
             start_date, end_date, end_reason, notes)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,
                 CASE WHEN $10::date IS NULL THEN NULL ELSE 'other' END, $11)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(from_contact)
    .bind(from_org)
    .bind(to_contact)
    .bind(to_org)
    .bind(from_role)
    .bind(to_role)
    .bind(date(row, "start_date"))
    .bind(date(row, "end_date"))
    .bind(optional(row, "notes"))
    .execute(&mut **tx)
    .await
    .map_err(|error| error.to_string())?;

    Ok(())
}

async fn lookup_contact(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    external_id: &str,
) -> Result<Option<Uuid>, String> {
    sqlx::query_scalar("SELECT id FROM contacts WHERE organisation_id=$1 AND external_id=$2")
        .bind(organisation)
        .bind(external_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|error| error.to_string())
}

async fn lookup_organisation(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    external_id: &str,
) -> Result<Option<Uuid>, String> {
    sqlx::query_scalar("SELECT id FROM organisations WHERE tenant_id=$1 AND external_id=$2")
        .bind(organisation)
        .bind(external_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_round_trip() {
        for object in ObjectType::ALL {
            let csv = template_csv(object);
            let parsed = parse(object, csv.as_bytes()).expect("parse template");
            assert!(parsed.errors.is_empty(), "{object:?}: {:?}", parsed.errors);
            assert_eq!(parsed.rows.len(), 1);
        }
    }

    #[test]
    fn rejects_unknown_columns() {
        let csv = "external_id,mystery\nX-1,hello\n";
        let parsed = parse(ObjectType::Contact, csv.as_bytes()).unwrap();
        assert!(parsed
            .errors
            .iter()
            .any(|error| error.message.contains("Unknown column")));
    }

    #[test]
    fn rejects_bad_values() {
        let csv = "external_id,display_name,email,date_of_birth,is_active\nC-1,Test,not-an-email,01/01/2000,maybe\n";
        let parsed = parse(ObjectType::Contact, csv.as_bytes()).unwrap();
        assert!(parsed.errors.len() >= 3);
    }

    #[test]
    fn stores_and_loads_rows() {
        let mut row = Row::new();
        row.insert("external_id".to_owned(), "C-1".to_owned());
        let stored = store_rows(&[row]);
        let loaded = load_rows(&stored);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0]["external_id"], "C-1");
    }
}
