//! Read-only PostgreSQL schema introspection for the database explorer.
//!
//! This deliberately reads **catalog metadata only** — tables, columns,
//! constraints, indexes and foreign keys. No domain rows are queried.

use std::collections::HashMap;

use axum::{
    extract::State,
    response::{Html, IntoResponse, Response},
};
use sqlx::{FromRow, PgPool};

use crate::{
    admin::{auth::AdminSession, erd, html},
    permissions::Capability,
    state::AppState,
};

#[derive(Debug, Clone, Default)]
pub struct SchemaModel {
    pub tables: Vec<TableModel>,
}

#[derive(Debug, Clone, Default)]
pub struct TableModel {
    pub name: String,
    pub comment: Option<String>,
    pub columns: Vec<ColumnModel>,
    pub primary_key: Vec<String>,
    pub foreign_keys: Vec<ForeignKey>,
    pub indexes: Vec<IndexModel>,
}

#[derive(Debug, Clone)]
pub struct ColumnModel {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub identity: bool,
}

#[derive(Debug, Clone)]
pub struct ForeignKey {
    pub column: String,
    pub target_table: String,
    pub target_column: String,
}

#[derive(Debug, Clone)]
pub struct IndexModel {
    pub name: String,
    pub definition: String,
    pub unique: bool,
}

#[derive(Debug, FromRow)]
struct ColumnRow {
    table_name: String,
    column_name: String,
    data_type: String,
    udt_name: String,
    is_nullable: String,
    column_default: Option<String>,
    is_identity: String,
}

#[derive(Debug, FromRow)]
struct ForeignKeyRow {
    source_table: String,
    source_column: String,
    target_table: String,
    target_column: String,
}

impl SchemaModel {
    pub async fn load(db: &PgPool) -> Result<Self, sqlx::Error> {
        let table_rows = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT c.relname::text, obj_description(c.oid, 'pg_class')
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = 'public' AND c.relkind = 'r'
               AND c.relname <> '_sqlx_migrations'
             ORDER BY c.relname",
        )
        .fetch_all(db)
        .await?;

        let column_rows = sqlx::query_as::<_, ColumnRow>(
            "SELECT table_name, column_name, data_type, udt_name,
                    is_nullable, column_default, is_identity
             FROM information_schema.columns
             WHERE table_schema = 'public'
             ORDER BY table_name, ordinal_position",
        )
        .fetch_all(db)
        .await?;

        let pk_rows = sqlx::query_as::<_, (String, String)>(
            "SELECT tc.table_name, kcu.column_name
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
               ON kcu.constraint_name = tc.constraint_name
              AND kcu.table_schema = tc.table_schema
             WHERE tc.constraint_type = 'PRIMARY KEY'
               AND tc.table_schema = 'public'
             ORDER BY tc.table_name, kcu.ordinal_position",
        )
        .fetch_all(db)
        .await?;

        let fk_rows = sqlx::query_as::<_, ForeignKeyRow>(
            "SELECT src.relname::text AS source_table,
                    src_col.attname::text AS source_column,
                    tgt.relname::text AS target_table,
                    tgt_col.attname::text AS target_column
             FROM pg_constraint con
             JOIN pg_class src ON src.oid = con.conrelid
             JOIN pg_class tgt ON tgt.oid = con.confrelid
             JOIN pg_namespace n ON n.oid = src.relnamespace
             CROSS JOIN LATERAL unnest(con.conkey, con.confkey)
                 WITH ORDINALITY AS pair(src_attnum, tgt_attnum, ord)
             JOIN pg_attribute src_col
               ON src_col.attrelid = con.conrelid AND src_col.attnum = pair.src_attnum
             JOIN pg_attribute tgt_col
               ON tgt_col.attrelid = con.confrelid AND tgt_col.attnum = pair.tgt_attnum
             WHERE con.contype = 'f'
               AND n.nspname = 'public'
               -- Tenant columns are implied by every table; hiding them keeps
               -- the model readable while the constraint still appears on the
               -- column badge and in the table definition.
               AND src_col.attname <> 'organisation_id'
               -- Composite FKs repeat the join column (shift_id ->
               -- shift_sessions.shift_id); drop that half of the constraint.
               AND NOT (src_col.attname = tgt_col.attname AND tgt_col.attname <> 'id')
             ORDER BY src.relname, con.conname, pair.ord",
        )
        .fetch_all(db)
        .await?;

        let index_rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT tablename, indexname, indexdef
             FROM pg_indexes
             WHERE schemaname = 'public'
             ORDER BY tablename, indexname",
        )
        .fetch_all(db)
        .await?;

        let mut tables: Vec<TableModel> = table_rows
            .into_iter()
            .map(|(name, comment)| TableModel {
                name,
                comment,
                ..TableModel::default()
            })
            .collect();

        let index: HashMap<String, usize> = tables
            .iter()
            .enumerate()
            .map(|(position, table)| (table.name.clone(), position))
            .collect();

        for row in column_rows {
            if let Some(position) = index.get(&row.table_name) {
                tables[*position].columns.push(ColumnModel {
                    name: row.column_name,
                    type_name: short_type(&row.data_type, &row.udt_name),
                    nullable: row.is_nullable == "YES",
                    default: row.column_default,
                    identity: row.is_identity == "YES",
                });
            }
        }

        for (table, column) in pk_rows {
            if let Some(position) = index.get(&table) {
                tables[*position].primary_key.push(column);
            }
        }

        for row in fk_rows {
            if let Some(position) = index.get(&row.source_table) {
                tables[*position].foreign_keys.push(ForeignKey {
                    column: row.source_column,
                    target_table: row.target_table,
                    target_column: row.target_column,
                });
            }
        }

        for (table, name, definition) in index_rows {
            if let Some(position) = index.get(&table) {
                tables[*position].indexes.push(IndexModel {
                    unique: definition.contains("UNIQUE INDEX"),
                    name,
                    definition,
                });
            }
        }

        Ok(Self { tables })
    }
}

fn short_type(data_type: &str, udt_name: &str) -> String {
    match data_type {
        "character varying" => "varchar".to_owned(),
        "timestamp with time zone" => "timestamptz".to_owned(),
        "timestamp without time zone" => "timestamp".to_owned(),
        "USER-DEFINED" => udt_name.to_owned(),
        "ARRAY" => format!("{}[]", udt_name.trim_start_matches('_')),
        other => other.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

pub async fn page(State(state): State<AppState>, session: AdminSession) -> Response {
    if let Some(response) = session.guard(Capability::SchemaView) {
        return response;
    }

    let model = match SchemaModel::load(&state.db).await {
        Ok(model) => model,
        Err(err) => {
            tracing::error!(error = %err, "schema introspection failed");
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Html(html::error_page(
                    "Database explorer",
                    "Could not read the database catalog. Check the server logs.",
                )),
            )
                .into_response();
        }
    };

    Html(render(&model, &session)).into_response()
}

fn render(model: &SchemaModel, session: &AdminSession) -> String {
    let mut body = String::new();

    body.push_str(&html::page_head(
        "Database explorer",
        "Structural map of the PostgreSQL schema. This page reads catalog metadata only — never table rows.",
    ));

    body.push_str(&format!(
        "<p class=\"sub\">{} tables · {} foreign keys</p>",
        model.tables.len(),
        model
            .tables
            .iter()
            .map(|table| table.foreign_keys.len())
            .sum::<usize>()
    ));

    body.push_str("<h2>Entity relationships</h2>");
    body.push_str("<div class=\"erd\">");
    body.push_str(&erd::render(model));
    body.push_str("</div>");
    body.push_str(
        "<p class=\"sub\">● primary key &nbsp; <span style=\"color:var(--warn)\">●</span> foreign key &nbsp; arrows point to the referenced table.</p>",
    );

    body.push_str("<h2>Relationships</h2>");
    if model
        .tables
        .iter()
        .all(|table| table.foreign_keys.is_empty())
    {
        body.push_str("<p class=\"sub\">No foreign keys defined.</p>");
    } else {
        body.push_str(
            "<table><thead><tr><th>From</th><th>To</th><th>Column</th></tr></thead><tbody>",
        );
        for table in &model.tables {
            for foreign_key in &table.foreign_keys {
                body.push_str(&format!(
                    "<tr><td><code>{}</code></td><td><code>{}.{}</code></td><td><code>{}</code></td></tr>",
                    html::escape(&format!("{}.{}", table.name, foreign_key.column)),
                    html::escape(&foreign_key.target_table),
                    html::escape(&foreign_key.target_column),
                    html::escape(&foreign_key.column),
                ));
            }
        }
        body.push_str("</tbody></table>");
    }

    body.push_str("<h2>Tables</h2>");
    for table in &model.tables {
        body.push_str("<div class=\"schema-card\"><h3>");
        body.push_str(&html::escape(&table.name));
        body.push_str("</h3>");
        if let Some(comment) = &table.comment {
            body.push_str(&format!("<p class=\"sub\">{}</p>", html::escape(comment)));
        }

        body.push_str("<table><thead><tr><th>Column</th><th>Type</th><th>Null</th><th>Default</th><th>Keys</th></tr></thead><tbody>");
        for column in &table.columns {
            let mut keys = String::new();
            if table.primary_key.contains(&column.name) {
                keys.push_str("<span class=\"badge pk\">PK</span> ");
            }
            if table
                .foreign_keys
                .iter()
                .any(|foreign_key| foreign_key.column == column.name)
            {
                keys.push_str("<span class=\"badge fk\">FK</span> ");
            }
            if column.identity {
                keys.push_str("<span class=\"badge id\">identity</span>");
            }

            body.push_str(&format!(
                "<tr><td><code>{}</code></td><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td></tr>",
                html::escape(&column.name),
                html::escape(&column.type_name),
                if column.nullable {
                    "<span class=\"muted\">yes</span>"
                } else {
                    "<span class=\"badge yes\">no</span>"
                },
                column
                    .default
                    .as_deref()
                    .map(|value| format!("<code>{}</code>", html::escape(&truncate(value, 48))))
                    .unwrap_or_else(|| "<span class=\"muted\">—</span>".to_owned()),
                keys,
            ));
        }
        body.push_str("</tbody></table>");

        if !table.indexes.is_empty() {
            body.push_str("<ul>");
            for index in &table.indexes {
                body.push_str(&format!(
                    "<li><code>{}</code>{}{}</li>",
                    html::escape(&index.name),
                    if index.unique {
                        " <span class=\"badge pk\">unique</span>"
                    } else {
                        ""
                    },
                    ""
                ));
            }
            body.push_str("</ul>");
        }

        body.push_str("</div>");
    }

    html::layout("Database", "/admin/schema", Some(&session.who()), &body)
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_owned()
    } else {
        let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_postgres_types_to_short_names() {
        assert_eq!(short_type("character varying", "varchar"), "varchar");
        assert_eq!(
            short_type("timestamp with time zone", "timestamptz"),
            "timestamptz"
        );
        assert_eq!(short_type("uuid", "uuid"), "uuid");
        assert_eq!(short_type("USER-DEFINED", "role_enum"), "role_enum");
        assert_eq!(short_type("ARRAY", "_text"), "text[]");
    }

    #[test]
    fn truncates_long_values() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("0123456789abcdef", 10), "012345678…");
    }
}
