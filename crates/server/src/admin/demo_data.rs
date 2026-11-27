//! Demo data: rebuild the Blue Dreams Care demonstration dataset.
//!
//! Seeding is destructive for the demo tenant and nothing else. The page shows
//! exactly what will be deleted, requires an explicit typed confirmation and is
//! disabled in production unless `DEMO_SEED_ENABLED=true` is set.

use std::time::Duration;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;

use crate::{
    admin::{auth::AdminSession, csrf, html},
    demo::{self, DemoTenant},
    permissions::Capability,
    state::AppState,
};

/// The seed can take a few seconds on a large database; treat anything beyond
/// this as hung rather than leaving the request open forever.
const SEED_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Default, Deserialize)]
pub struct PageParams {
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmForm {
    csrf: String,
    #[serde(default)]
    confirm: Option<String>,
    #[serde(default)]
    confirm_slug: Option<String>,
}

pub async fn page(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Query(params): Query<PageParams>,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }

    let tenant = match demo::status(&state.db).await {
        Ok(tenant) => tenant,
        Err(err) => {
            tracing::error!(error = %err, "demo tenant status query failed");
            return failure("The demo data status could not be loaded.");
        }
    };

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
    let body = render(Page {
        tenant: tenant.as_ref(),
        enabled: state.config.demo_seed_enabled,
        csrf: &token,
        error: params.error.as_deref().map(error_message),
    });

    let page = html::layout("Demo data", "/admin/demo-data", Some(&session.who()), &body);
    (jar, Html(page)).into_response()
}

pub async fn seed(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Form(form): Form<ConfirmForm>,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }

    if !state.config.demo_seed_enabled {
        return Redirect::to("/admin/demo-data?error=disabled").into_response();
    }

    if !confirmation_is_complete(form.confirm.as_deref(), form.confirm_slug.as_deref()) {
        return Redirect::to("/admin/demo-data?error=confirm").into_response();
    }

    let Ok(_guard) = state.demo_seed_lock.try_lock() else {
        return Redirect::to("/admin/demo-data?error=busy").into_response();
    };

    tracing::warn!(
        actor = %session.email,
        organisation = %session.organisation_id,
        "demo dataset reseed requested from the admin UI"
    );

    match tokio::time::timeout(SEED_TIMEOUT, demo::seed(&state.db)).await {
        Ok(Ok(report)) => {
            tracing::info!(
                organisation = %report.organisation_id,
                contacts = report.contacts,
                shifts = report.shifts,
                "demo dataset reseeded"
            );
            Html(html::layout("Demo data", "", None, &success_body(&report))).into_response()
        }
        Ok(Err(err)) => {
            tracing::error!(error = %err, "demo dataset reseed failed");
            failure_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The demo seed failed part-way through. The demo organisation may be incomplete; \
                 check the server logs and run it again.",
            )
        }
        Err(_) => {
            tracing::error!("demo dataset reseed timed out");
            failure_page(
                StatusCode::GATEWAY_TIMEOUT,
                "The demo seed took too long and was cancelled. Check the server logs and run it \
                 again.",
            )
        }
    }
}

/// Both the checkbox and the typed slug must match before anything is deleted.
fn confirmation_is_complete(confirm: Option<&str>, slug: Option<&str>) -> bool {
    confirm == Some("yes") && slug.map(str::trim) == Some(demo::SLUG)
}

struct Page<'a> {
    tenant: Option<&'a DemoTenant>,
    enabled: bool,
    csrf: &'a str,
    error: Option<&'a str>,
}

fn render(page: Page<'_>) -> String {
    let mut body = String::new();
    body.push_str(&html::page_head(
        "Demo data",
        "Rebuild the Blue Dreams Care demonstration dataset.",
    ));

    if let Some(error) = page.error {
        body.push_str(&format!(
            "<div class=\"alert\">{}</div>",
            html::escape(error)
        ));
    }

    body.push_str("<h2>What seeding does</h2>");
    body.push_str("<div class=\"card\">");
    body.push_str(
        "<p>Generates a complete demonstration tenant: clients, workers and their \
         households, relationships, rosters and shifts, worker qualities, funding \
         packages and agreements, custom fields, services, sites and demo logins.</p>",
    );
    body.push_str(
        "<p class=\"sub\">Only the <code>blue-dreams-care</code> organisation and the \
         <code>@bluedreams.example</code> demo users are replaced. Every other \
         organisation is left alone.</p>",
    );
    body.push_str("</div>");

    if !page.enabled {
        body.push_str(
            "<div class=\"alert\"><strong>Demo seeding is disabled on this server.</strong> \
             Set <code>DEMO_SEED_ENABLED=true</code> and restart the server to enable it.</div>",
        );
        return body;
    }

    match page.tenant {
        Some(tenant) => {
            body.push_str("<h2>Existing demo data</h2>");
            body.push_str(&format!(
                "<div class=\"danger-box\"><p><strong>Seeding permanently deletes the existing \
                 “{}” organisation and everything inside it.</strong> This cannot be undone.</p>",
                html::escape(&tenant.name)
            ));
            body.push_str("<table><thead><tr><th>Record</th><th>Deleted</th></tr></thead><tbody>");
            for (label, count) in [
                ("Contacts", tenant.contacts),
                ("Relationships", tenant.relationships),
                ("Shifts", tenant.shifts),
                ("Funding agreements", tenant.agreements),
                ("Demo users", tenant.users),
            ] {
                body.push_str(&format!("<tr><td>{label}</td><td>{count}</td></tr>"));
            }
            body.push_str("</tbody></table>");
            body.push_str(
                "<p class=\"sub\">Audit history for the demo organisation is deleted too. \
                 Data in every other organisation is untouched.</p></div>",
            );
        }
        None => {
            body.push_str(
                "<div class=\"notice\">No demo organisation exists yet. Seeding will create it.</div>",
            );
        }
    }

    body.push_str("<h2>Replace demo data</h2>");
    body.push_str("<div class=\"danger-box\">");
    body.push_str(
        "<p>This is a development and demonstration tool. Do not run it on a database \
         that holds real client records.</p>",
    );
    body.push_str(&format!(
        "<form class=\"form wide\" method=\"post\" action=\"/admin/demo-data\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        html::escape(page.csrf)
    ));
    body.push_str(
        "<div class=\"checkbox\"><input id=\"confirm\" name=\"confirm\" type=\"checkbox\" value=\"yes\">\
         <label for=\"confirm\">I understand the existing demo data will be permanently deleted.</label></div>",
    );
    body.push_str(&format!(
        "<label for=\"confirm_slug\">Type <code>{}</code> to confirm</label>",
        html::escape(demo::SLUG)
    ));
    body.push_str(
        "<input id=\"confirm_slug\" name=\"confirm_slug\" autocomplete=\"off\" \
         spellcheck=\"false\" required>",
    );
    body.push_str(
        "<p class=\"hint\">If you are signed in as a demo user, you will be signed out \
         because those accounts are recreated.</p>",
    );
    body.push_str(
        "<button type=\"submit\" class=\"danger\">Replace demo data</button></form></div>",
    );

    body
}

fn error_message(code: &str) -> &'static str {
    match code {
        "disabled" => "Demo seeding is disabled on this server.",
        "confirm" => "Nothing was changed: tick the box and type blue-dreams-care to confirm.",
        "busy" => "Another demo seed is already running. Wait for it to finish and try again.",
        _ => "The requested change could not be applied.",
    }
}

/// Rendered after a successful seed. Signed-in demo users have just been
/// deleted, so this page is public and hands them the new credentials.
fn success_body(report: &demo::SeedReport) -> String {
    let mut body = String::new();
    body.push_str(&html::page_head(
        "Demo data rebuilt",
        "The Blue Dreams Care demonstration tenant is ready.",
    ));
    body.push_str(
        "<div class=\"notice\"><strong>Seeded.</strong> The demo organisation and its demo \
         users were recreated. Data in every other organisation is untouched.</div>",
    );
    body.push_str("<div class=\"grid\">");
    for (label, value) in [
        ("Clients", report.clients as i64),
        ("Staff", report.staff as i64),
        ("Contacts", report.contacts),
        ("Relationships", report.relationships),
        ("Shifts", report.shifts),
        ("Intake cases", report.intake_cases as i64),
        ("Waitlisted", report.intake_waitlist as i64),
        ("Funding agreements", report.funding_agreements as i64),
    ] {
        body.push_str(&format!(
            "<div class=\"card\"><p class=\"sub\">{label}</p><p class=\"big\">{value}</p></div>"
        ));
    }
    body.push_str("</div>");
    body.push_str("<h2>Sign in</h2><div class=\"card\">");
    body.push_str(
        "<p>Demo users were recreated, so any session signed in as a demo account has ended.</p>",
    );
    body.push_str(&format!(
        "<p class=\"sub\"><code>{}</code> · <code>{}</code></p>",
        html::escape(demo::OWNER_EMAIL),
        html::escape(demo::PASSWORD)
    ));
    body.push_str(
        "<p><a class=\"button\" href=\"/admin/login\">Sign in</a> <a href=\"/admin\">Continue \
         to the dashboard</a></p></div>",
    );
    body
}

fn failure_page(status: StatusCode, message: &str) -> Response {
    let body = format!(
        "{}<div class=\"alert\">{}</div><p><a class=\"button\" href=\"/admin/demo-data\">Try again</a></p>",
        html::page_head("Demo seed failed", "The demo dataset could not be rebuilt."),
        html::escape(message)
    );
    (status, Html(html::layout("Demo data", "", None, &body))).into_response()
}

fn csrf_failure() -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(html::error_page(
            "Demo data",
            "The form expired. Reload the page and try again.",
        )),
    )
        .into_response()
}

fn failure(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page("Demo data", message)),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant() -> DemoTenant {
        DemoTenant {
            organisation_id: uuid::Uuid::nil(),
            name: "Blue Dreams Care".to_owned(),
            contacts: 128,
            relationships: 260,
            shifts: 342,
            agreements: 12,
            users: 5,
        }
    }

    #[test]
    fn confirmation_requires_the_box_and_the_exact_slug() {
        assert!(confirmation_is_complete(
            Some("yes"),
            Some("blue-dreams-care")
        ));
        assert!(confirmation_is_complete(
            Some("yes"),
            Some("  blue-dreams-care\n")
        ));
        assert!(!confirmation_is_complete(Some("yes"), Some("blue-dreams")));
        assert!(!confirmation_is_complete(
            Some("yes"),
            Some("BLUE-DREAMS-CARE")
        ));
        assert!(!confirmation_is_complete(Some("yes"), Some("")));
        assert!(!confirmation_is_complete(None, Some("blue-dreams-care")));
        assert!(!confirmation_is_complete(
            Some("on"),
            Some("blue-dreams-care")
        ));
    }

    #[test]
    fn page_warns_about_the_records_that_will_be_deleted() {
        let tenant = tenant();
        let body = render(Page {
            tenant: Some(&tenant),
            enabled: true,
            csrf: "token",
            error: None,
        });

        assert!(body.contains("permanently deletes the existing “Blue Dreams Care”"));
        assert!(body.contains("<tr><td>Contacts</td><td>128</td></tr>"));
        assert!(body.contains("<tr><td>Relationships</td><td>260</td></tr>"));
        assert!(body.contains("<tr><td>Shifts</td><td>342</td></tr>"));
        assert!(body.contains("<tr><td>Funding agreements</td><td>12</td></tr>"));
        assert!(body.contains("<tr><td>Demo users</td><td>5</td></tr>"));
        assert!(body.contains("type=\"checkbox\""));
        assert!(body.contains("Type <code>blue-dreams-care</code> to confirm"));
        assert!(body.contains("Data in every other organisation is untouched"));
        assert!(!body.contains("No demo organisation exists yet"));
    }

    #[test]
    fn empty_database_is_described_as_a_first_seed() {
        let body = render(Page {
            tenant: None,
            enabled: true,
            csrf: "token",
            error: None,
        });

        assert!(body.contains("No demo organisation exists yet"));
        assert!(!body.contains("permanently deletes"));
    }

    #[test]
    fn disabled_page_explains_the_flag_and_hides_the_form() {
        let tenant = tenant();
        let body = render(Page {
            tenant: Some(&tenant),
            enabled: false,
            csrf: "token",
            error: None,
        });

        assert!(body.contains("Demo seeding is disabled on this server"));
        assert!(body.contains("DEMO_SEED_ENABLED=true"));
        assert!(!body.contains("<form"));
        assert!(!body.contains("permanently deletes"));
    }

    #[test]
    fn confirmation_errors_are_explained() {
        let body = render(Page {
            tenant: None,
            enabled: true,
            csrf: "token",
            error: Some(error_message("confirm")),
        });

        assert!(body.contains("tick the box and type blue-dreams-care"));
    }

    #[test]
    fn success_page_reports_what_was_created_and_how_to_sign_in() {
        let report = demo::SeedReport {
            organisation_id: uuid::Uuid::nil(),
            clients: 14,
            staff: 9,
            duplicate_names: 1,
            contacts: 131,
            relationships: 262,
            shifts: 340,
            sites: 6,
            constraints: 18,
            quality_values: 40,
            client_requirements: 12,
            organisation_requirements: 5,
            funding_packages: 9,
            funding_agreements: 11,
            generated_shifts: 24,
            divisions: 3,
            cost_centres: 4,
            services: 12,
            intake_cases: 26,
            intake_waitlist: 3,
        };

        let body = success_body(&report);
        assert!(body.contains("Demo data rebuilt"));
        assert!(body.contains("<p class=\"big\">14</p>"));
        assert!(body.contains("<p class=\"big\">131</p>"));
        assert!(body.contains("<p class=\"big\">262</p>"));
        assert!(body.contains("<p class=\"big\">340</p>"));
        assert!(body.contains("owner@bluedreams.example"));
        assert!(body.contains("BlueDreamsCare!2026"));
        assert!(body.contains("href=\"/admin/login\""));
    }
}
