//! Organisation profile and branding.
//!
//! The name lives on `organisations`. The logo is stored in
//! `organisation_branding` and served with a SHA-256 ETag so desktop and mobile
//! clients can cache it.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use utoipa::ToSchema;
use uuid::Uuid;

/// Maximum accepted logo size in bytes.
pub const MAX_LOGO_BYTES: usize = 2 * 1024 * 1024;

/// Path clients use to fetch the logo bytes.
pub const LOGO_PATH: &str = "/api/v1/organisation/logo";

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LogoInfo {
    pub content_type: String,
    pub filename: Option<String>,
    pub sha256: String,
    pub updated_at: DateTime<Utc>,
    /// Relative URL clients can call with their bearer token.
    pub url: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OrganisationProfileDto {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub edition: String,
    pub logo: Option<LogoInfo>,
}

#[derive(Debug, Clone)]
pub struct LogoBlob {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub sha256: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct ProfileRow {
    id: Uuid,
    slug: String,
    name: String,
    edition: String,
    logo_content_type: Option<String>,
    logo_filename: Option<String>,
    logo_sha256: Option<String>,
    logo_updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct LogoRow {
    logo: Vec<u8>,
    logo_content_type: String,
    logo_sha256: String,
    updated_at: DateTime<Utc>,
}

/// Validate an uploaded logo and return its detected MIME type.
pub fn validate_logo(bytes: &[u8]) -> Result<&'static str, String> {
    if bytes.is_empty() {
        return Err("the logo file is empty".to_owned());
    }
    if bytes.len() > MAX_LOGO_BYTES {
        return Err("the logo must be 2 MB or smaller".to_owned());
    }
    detect_content_type(bytes)
        .ok_or_else(|| "the logo must be a PNG, JPEG, WebP or GIF image".to_owned())
}

/// Detect a supported image type from magic bytes.
#[must_use]
pub fn detect_content_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

pub async fn load_profile(
    db: &PgPool,
    organisation_id: Uuid,
) -> Result<Option<OrganisationProfileDto>, sqlx::Error> {
    let row = sqlx::query_as::<_, ProfileRow>(
        "SELECT o.id, COALESCE(o.slug, '') AS slug, o.name, o.edition,
                b.logo_content_type, b.logo_filename, b.logo_sha256,
                b.updated_at AS logo_updated_at
         FROM organisations o
         LEFT JOIN organisation_branding b ON b.organisation_id = o.id
         WHERE o.id = $1 AND o.tenant_id = o.id",
    )
    .bind(organisation_id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|row| {
        let logo = match (row.logo_content_type, row.logo_sha256, row.logo_updated_at) {
            (Some(content_type), Some(sha256), Some(updated_at)) => Some(LogoInfo {
                content_type,
                filename: row.logo_filename,
                sha256,
                updated_at,
                url: LOGO_PATH.to_owned(),
            }),
            _ => None,
        };

        OrganisationProfileDto {
            id: row.id,
            slug: row.slug,
            name: row.name,
            edition: row.edition,
            logo,
        }
    }))
}

pub async fn update_name(
    db: &PgPool,
    organisation_id: Uuid,
    name: &str,
) -> Result<u64, sqlx::Error> {
    Ok(
        sqlx::query("UPDATE organisations SET name = $1 WHERE id = $2 AND tenant_id = id")
            .bind(name)
            .bind(organisation_id)
            .execute(db)
            .await?
            .rows_affected(),
    )
}

pub async fn load_logo(
    db: &PgPool,
    organisation_id: Uuid,
) -> Result<Option<LogoBlob>, sqlx::Error> {
    let row = sqlx::query_as::<_, LogoRow>(
        "SELECT logo, logo_content_type, logo_sha256, updated_at
         FROM organisation_branding WHERE organisation_id = $1",
    )
    .bind(organisation_id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|row| LogoBlob {
        bytes: row.logo,
        content_type: row.logo_content_type,
        sha256: row.logo_sha256,
        updated_at: row.updated_at,
    }))
}

pub async fn save_logo(
    db: &PgPool,
    organisation_id: Uuid,
    updated_by: Uuid,
    bytes: &[u8],
    content_type: &str,
    filename: Option<&str>,
) -> Result<LogoInfo, sqlx::Error> {
    let sha256 = hex::encode(Sha256::digest(bytes));
    let updated_at = Utc::now();

    sqlx::query(
        "INSERT INTO organisation_branding
            (organisation_id, logo, logo_content_type, logo_filename, logo_sha256,
             updated_by, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (organisation_id) DO UPDATE SET
            logo = excluded.logo,
            logo_content_type = excluded.logo_content_type,
            logo_filename = excluded.logo_filename,
            logo_sha256 = excluded.logo_sha256,
            updated_by = excluded.updated_by,
            updated_at = excluded.updated_at",
    )
    .bind(organisation_id)
    .bind(bytes)
    .bind(content_type)
    .bind(filename)
    .bind(&sha256)
    .bind(updated_by)
    .bind(updated_at)
    .execute(db)
    .await?;

    Ok(LogoInfo {
        content_type: content_type.to_owned(),
        filename: filename.map(str::to_owned),
        sha256,
        updated_at,
        url: LOGO_PATH.to_owned(),
    })
}

pub async fn delete_logo(db: &PgPool, organisation_id: Uuid) -> Result<u64, sqlx::Error> {
    Ok(
        sqlx::query("DELETE FROM organisation_branding WHERE organisation_id = $1")
            .bind(organisation_id)
            .execute(db)
            .await?
            .rows_affected(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_image_types() {
        assert_eq!(
            detect_content_type(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
            Some("image/png")
        );
        assert_eq!(
            detect_content_type(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some("image/jpeg")
        );
        assert_eq!(detect_content_type(b"GIF89a..."), Some("image/gif"));
        assert_eq!(
            detect_content_type(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(detect_content_type(b"not an image"), None);
    }

    #[test]
    fn rejects_empty_unknown_and_oversized_logos() {
        assert!(validate_logo(&[]).is_err());
        assert!(validate_logo(b"not an image").is_err());
        assert!(validate_logo(&vec![0u8; MAX_LOGO_BYTES + 1]).is_err());
    }
}
