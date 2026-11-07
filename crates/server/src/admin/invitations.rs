//! One-time invitation links.
//!
//! Admin side creates an invitation and sees the link exactly once. The public
//! acceptance page creates a new account, applies the invitation's permission
//! sets and signs the user in. Existing accounts are refused in v1.

use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    admin::{
        auth::{session_cookie, start_session, write_audit, AdminSession},
        csrf, html,
    },
    auth::{password, refresh},
    permissions::CapabilitySet,
    state::AppState,
};

const MAX_EXPIRY_DAYS: i64 = 30;

#[derive(Debug, FromRow)]
struct InvitationRow {
    id: Uuid,
    email: String,
    full_name: Option<String>,
    expires_at: DateTime<Utc>,
    accepted_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    permission_sets: String,
}

#[derive(Debug, Clone)]
struct SetOption {
    id: Uuid,
    name: String,
    description: Option<String>,
    capabilities: CapabilitySet,
}

#[derive(Debug, FromRow)]
struct SetCapabilityRow {
    id: Uuid,
    name: String,
    description: Option<String>,
    capability: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AcceptForm {
    password: String,
    confirmation: String,
}

#[derive(Debug, FromRow)]
struct AcceptInvitation {
    id: Uuid,
    organisation_id: Uuid,
    email: String,
    full_name: Option<String>,
    expires_at: DateTime<Utc>,
    accepted_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    organisation_name: String,
    permission_sets: String,
}

// ---------------------------------------------------------------------------
// Admin list and creation
// ---------------------------------------------------------------------------

pub async fn list(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(crate::permissions::Capability::UsersManage) {
        return response;
    }

    let rows = sqlx::query_as::<_, InvitationRow>(
        "SELECT i.id, i.email, i.full_name, i.expires_at, i.accepted_at, i.revoked_at,
                i.created_at,
                COALESCE(string_agg(ps.name, ', ' ORDER BY ps.name), '') AS permission_sets
         FROM invitations i
         LEFT JOIN invitation_permission_sets ips ON ips.invitation_id = i.id
         LEFT JOIN permission_sets ps ON ps.id = ips.permission_set_id
         WHERE i.organisation_id = $1
         GROUP BY i.id
         ORDER BY i.created_at DESC",
    )
    .bind(session.organisation_id)
    .fetch_all(&state.db)
    .await;

    let rows = match rows {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(error = %err, "invitation list failed");
            return failure("Invitations could not be loaded.");
        }
    };

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
    let now = Utc::now();

    let mut body = String::new();
    body.push_str(&html::page_head(
        "Invitations",
        "One-time links. The link is shown once and grants the selected permission sets.",
    ));
    body.push_str("<p><a class=\"button\" href=\"/admin/invitations/new\">Invite a user</a></p>");

    if rows.is_empty() {
        body.push_str("<p class=\"sub\">No invitations yet.</p>");
    } else {
        body.push_str("<table><thead><tr><th>Email</th><th>Name</th><th>Permission sets</th><th>Status</th><th>Created (UTC)</th><th>Expires (UTC)</th><th></th></tr></thead><tbody>");
        for row in &rows {
            let status = if row.revoked_at.is_some() {
                "<span class=\"badge yes\">revoked</span>"
            } else if row.accepted_at.is_some() {
                "<span class=\"badge pk\">accepted</span>"
            } else if row.expires_at <= now {
                "<span class=\"badge yes\">expired</span>"
            } else {
                "<span class=\"badge id\">pending</span>"
            };

            let actions =
                if row.accepted_at.is_none() && row.revoked_at.is_none() && row.expires_at > now {
                    format!(
                        "<form method=\"post\" action=\"/admin/invitations/{}/revoke\">\
                     <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
                     <button class=\"link-button\" type=\"submit\">Revoke</button></form>",
                        row.id,
                        html::escape(&token)
                    )
                } else {
                    String::new()
                };

            body.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"muted\">{}</td><td class=\"muted\">{}</td><td>{}</td></tr>",
                html::escape(&row.email),
                html::escape(row.full_name.as_deref().unwrap_or("—")),
                html::escape(&row.permission_sets),
                status,
                row.created_at.format("%Y-%m-%d %H:%M"),
                row.expires_at.format("%Y-%m-%d %H:%M"),
                actions,
            ));
        }
        body.push_str("</tbody></table>");
    }

    let html_body = html::layout(
        "Invitations",
        "/admin/invitations",
        Some(&session.who()),
        &body,
    );
    (jar, Html(html_body)).into_response()
}

pub async fn new_form(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(crate::permissions::Capability::UsersManage) {
        return response;
    }

    let sets = match grantable_sets(&state, &session).await {
        Ok(sets) => sets,
        Err(err) => {
            tracing::error!(error = %err, "permission set lookup failed");
            return failure("Permission sets could not be loaded.");
        }
    };

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
    let body = render_new(NewPage {
        session: &session,
        csrf: &token,
        sets: &sets,
        values: NewValues::default(),
        errors: Vec::new(),
    });

    (jar, Html(body)).into_response()
}

pub async fn create(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(crate::permissions::Capability::UsersManage) {
        return response;
    }

    let csrf_token = form.get("csrf").cloned().unwrap_or_default();
    if !csrf::verify(&jar, &csrf_token) {
        return csrf_failure();
    }

    let sets = match grantable_sets(&state, &session).await {
        Ok(sets) => sets,
        Err(err) => {
            tracing::error!(error = %err, "permission set lookup failed");
            return failure("Permission sets could not be loaded.");
        }
    };

    let values = NewValues::from_form(&form);
    let selected = selected_sets(&form);

    let errors = validate_new(&state, &values, &selected, &sets).await;
    if !errors.is_empty() {
        let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
        let body = render_new(NewPage {
            session: &session,
            csrf: &token,
            sets: &sets,
            values,
            errors,
        });
        return (StatusCode::UNPROCESSABLE_ENTITY, jar, Html(body)).into_response();
    }

    let email = values.email.trim().to_ascii_lowercase();
    let (token, token_hash) = refresh::generate();
    let expiry_days = values.expiry_days();
    let invitation_id = Uuid::now_v7();

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, "invitation transaction failed");
            return failure("The invitation could not be created.");
        }
    };

    if let Err(err) = sqlx::query(
        "INSERT INTO invitations
            (id, organisation_id, email, full_name, token_hash, expires_at,
             created_by)
         VALUES ($1, $2, $3, $4, $5, now() + make_interval(days => $6), $7)",
    )
    .bind(invitation_id)
    .bind(session.organisation_id)
    .bind(&email)
    .bind(trim_optional(&values.full_name))
    .bind(&token_hash)
    .bind(i32::try_from(expiry_days).unwrap_or(7))
    .bind(session.user_id)
    .execute(&mut *tx)
    .await
    {
        tracing::warn!(error = %err, "invitation insert failed");
        let message = if is_unique_violation(&err) {
            "A pending invitation already exists for that email."
        } else {
            "The invitation could not be created."
        };
        let (jar, csrf_token) = csrf::ensure(jar, state.config.environment.is_production());
        let body = render_new(NewPage {
            session: &session,
            csrf: &csrf_token,
            sets: &sets,
            values,
            errors: vec![message.to_owned()],
        });
        return (StatusCode::UNPROCESSABLE_ENTITY, jar, Html(body)).into_response();
    }

    for set_id in &selected {
        if let Err(err) = sqlx::query(
            "INSERT INTO invitation_permission_sets
                (organisation_id, invitation_id, permission_set_id)
             VALUES ($1, $2, $3)",
        )
        .bind(session.organisation_id)
        .bind(invitation_id)
        .bind(set_id)
        .execute(&mut *tx)
        .await
        {
            tracing::error!(error = %err, "invitation set insert failed");
            return failure("The invitation could not be created.");
        }
    }

    if let Err(err) = tx.commit().await {
        tracing::error!(error = %err, "invitation commit failed");
        return failure("The invitation could not be created.");
    }

    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "invitation.created",
        "invitation",
        Some(invitation_id),
    )
    .await;

    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("127.0.0.1:{}", state.config.bind_addr.port()));
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .filter(|value| *value == "http" || *value == "https")
        .unwrap_or("http");
    let link = format!("{scheme}://{host}/invite/{token}");

    let body = link_page(&session, &email, &values.full_name, &selected, &sets, &link);
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Html(body),
    )
        .into_response()
}

pub async fn revoke(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(crate::permissions::Capability::UsersManage) {
        return response;
    }

    let token = form.get("csrf").cloned().unwrap_or_default();
    if !csrf::verify(&jar, &token) {
        return csrf_failure();
    }

    if let Err(err) = sqlx::query(
        "UPDATE invitations SET revoked_at = now()
         WHERE id = $1 AND organisation_id = $2 AND accepted_at IS NULL",
    )
    .bind(id)
    .bind(session.organisation_id)
    .execute(&state.db)
    .await
    {
        tracing::error!(error = %err, "invitation revoke failed");
        return failure("The invitation could not be revoked.");
    }

    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "invitation.revoked",
        "invitation",
        Some(id),
    )
    .await;

    (jar, Redirect::to("/admin/invitations")).into_response()
}

// ---------------------------------------------------------------------------
// Public acceptance
// ---------------------------------------------------------------------------

pub async fn accept_form(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    match find_invitation(&state, &token).await {
        Ok(Some(invitation)) if invitation_is_valid(&invitation) => (
            invite_headers(),
            Html(accept_page(&invitation, &token, None)),
        )
            .into_response(),
        Ok(_) | Err(_) => (invite_headers(), Html(invalid_page())).into_response(),
    }
}

pub async fn accept(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    Form(form): Form<AcceptForm>,
) -> Response {
    let invitation = match find_invitation(&state, &token).await {
        Ok(Some(invitation)) if invitation_is_valid(&invitation) => invitation,
        Ok(_) | Err(_) => return (invite_headers(), Html(invalid_page())).into_response(),
    };

    if form.password != form.confirmation {
        return (
            invite_headers(),
            Html(accept_page(
                &invitation,
                &token,
                Some("Passwords do not match."),
            )),
        )
            .into_response();
    }
    if form.password.chars().count() < 12 {
        return (
            invite_headers(),
            Html(accept_page(
                &invitation,
                &token,
                Some("Password must be at least 12 characters."),
            )),
        )
            .into_response();
    }

    let email = invitation.email.trim().to_ascii_lowercase();
    let password_hash = match password::hash_password(form.password).await {
        Ok(hash) => hash,
        Err(err) => {
            tracing::error!(error = ?err, "invitation password hashing failed");
            return (
                invite_headers(),
                Html(accept_page(
                    &invitation,
                    &token,
                    Some("The account could not be created. Try again."),
                )),
            )
                .into_response();
        }
    };

    let user_id = Uuid::now_v7();
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, "invitation acceptance transaction failed");
            return failure("The invitation could not be accepted.");
        }
    };

    // Lock the invitation so concurrent accepts cannot both succeed.
    let current =
        sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT accepted_at, revoked_at, expires_at FROM invitations
         WHERE id = $1 FOR UPDATE",
        )
        .bind(invitation.id)
        .fetch_one(&mut *tx)
        .await;

    match current {
        Ok((None, None, expires_at)) if expires_at > Utc::now() => {}
        _ => {
            return (invite_headers(), Html(invalid_page())).into_response();
        }
    }

    let existing: bool =
        match sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE lower(email) = $1)")
            .bind(&email)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(exists) => exists,
            Err(err) => {
                tracing::error!(error = %err, "invitation email check failed");
                return failure("The invitation could not be accepted.");
            }
        };

    if existing {
        return (
            invite_headers(),
            Html(accept_page(
                &invitation,
                &token,
                Some(
                    "This email already has an account. Sign in, then ask an administrator to add you.",
                ),
            )),
        )
            .into_response();
    }

    let insert_user = sqlx::query(
        "INSERT INTO users (id, email, password_hash, full_name) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(&email)
    .bind(&password_hash)
    .bind(invitation.full_name.as_deref().unwrap_or(""))
    .execute(&mut *tx)
    .await;

    if let Err(err) = insert_user {
        tracing::error!(error = %err, "invitation user insert failed");
        return failure("The invitation could not be accepted.");
    }

    if let Err(err) = sqlx::query(
        "INSERT INTO organisation_users (organisation_id, user_id, role, is_owner)
         VALUES ($1, $2, 'staff', false)",
    )
    .bind(invitation.organisation_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    {
        tracing::error!(error = %err, "invitation membership insert failed");
        return failure("The invitation could not be accepted.");
    }

    if let Err(err) = sqlx::query(
        "INSERT INTO organisation_user_permission_sets
            (organisation_id, user_id, permission_set_id)
         SELECT organisation_id, $2, permission_set_id
         FROM invitation_permission_sets WHERE invitation_id = $1",
    )
    .bind(invitation.id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    {
        tracing::error!(error = %err, "invitation set assignment failed");
        return failure("The invitation could not be accepted.");
    }

    if let Err(err) = sqlx::query(
        "UPDATE invitations
         SET accepted_at = now(), accepted_user_id = $2
         WHERE id = $1 AND accepted_at IS NULL",
    )
    .bind(invitation.id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    {
        tracing::error!(error = %err, "invitation acceptance update failed");
        return failure("The invitation could not be accepted.");
    }

    if let Err(err) = tx.commit().await {
        tracing::error!(error = %err, "invitation acceptance commit failed");
        return failure("The invitation could not be accepted.");
    }

    write_audit(
        &state.db,
        invitation.organisation_id,
        user_id,
        "invitation.accepted",
        "invitation",
        Some(invitation.id),
    )
    .await;

    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(512).collect::<String>());

    match start_session(&state, user_id, invitation.organisation_id, user_agent).await {
        Ok(session_token) => {
            let cookie = session_cookie(&state, session_token);
            (jar.add(cookie), Redirect::to("/admin")).into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, "invitation session creation failed");
            // The account exists; send them to the normal sign-in page.
            Redirect::to("/admin/login").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn find_invitation(
    state: &AppState,
    token: &str,
) -> Result<Option<AcceptInvitation>, sqlx::Error> {
    sqlx::query_as::<_, AcceptInvitation>(
        "SELECT i.id, i.organisation_id, i.email, i.full_name, i.expires_at,
                i.accepted_at, i.revoked_at,
                o.name AS organisation_name,
                COALESCE(string_agg(ps.name, ', ' ORDER BY ps.name), '') AS permission_sets
         FROM invitations i
         JOIN organisations o ON o.id = i.organisation_id
         LEFT JOIN invitation_permission_sets ips ON ips.invitation_id = i.id
         LEFT JOIN permission_sets ps ON ps.id = ips.permission_set_id
         WHERE i.token_hash = $1
         GROUP BY i.id, o.name",
    )
    .bind(refresh::hash(token))
    .fetch_optional(&state.db)
    .await
}

fn invitation_is_valid(invitation: &AcceptInvitation) -> bool {
    invitation.accepted_at.is_none()
        && invitation.revoked_at.is_none()
        && invitation.expires_at > Utc::now()
}

async fn grantable_sets(
    state: &AppState,
    session: &AdminSession,
) -> Result<Vec<SetOption>, sqlx::Error> {
    let rows = sqlx::query_as::<_, SetCapabilityRow>(
        "SELECT ps.id, ps.name, ps.description, c.capability
         FROM permission_sets ps
         LEFT JOIN permission_set_capabilities c ON c.permission_set_id = ps.id
         WHERE ps.organisation_id = $1
         ORDER BY ps.sort_order, ps.name",
    )
    .bind(session.organisation_id)
    .fetch_all(&state.db)
    .await?;

    let mut sets: Vec<SetOption> = Vec::new();
    for row in rows {
        let position = match sets.iter().position(|set| set.id == row.id) {
            Some(position) => position,
            None => {
                sets.push(SetOption {
                    id: row.id,
                    name: row.name,
                    description: row.description,
                    capabilities: CapabilitySet::empty(),
                });
                sets.len() - 1
            }
        };

        if let Some(key) = row.capability {
            if let Some(capability) = crate::permissions::Capability::parse(&key) {
                sets[position].capabilities.insert(capability);
            }
        }
    }

    if session.is_owner {
        Ok(sets)
    } else {
        // Never allow an inviter to grant capabilities they do not hold.
        Ok(sets
            .into_iter()
            .filter(|set| set.capabilities.is_subset_of(session.capabilities))
            .collect())
    }
}

fn selected_sets(form: &HashMap<String, String>) -> Vec<Uuid> {
    form.keys()
        .filter_map(|key| key.strip_prefix("set_"))
        .filter_map(|raw| raw.parse::<Uuid>().ok())
        .collect()
}

async fn validate_new(
    state: &AppState,
    values: &NewValues,
    selected: &[Uuid],
    sets: &[SetOption],
) -> Vec<String> {
    let mut errors = Vec::new();

    let email = values.email.trim().to_ascii_lowercase();
    let valid_email = match email.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
        }
        None => false,
    };
    if !valid_email || email.len() > 320 {
        errors.push("Enter a valid email address.".to_owned());
    }

    if selected.is_empty() {
        errors.push("Choose at least one permission set.".to_owned());
    }

    for set_id in selected {
        if !sets.iter().any(|set| &set.id == set_id) {
            errors.push("One of the selected sets cannot be granted by your account.".to_owned());
        }
    }

    let days = values.expiry_days();
    if !(1..=MAX_EXPIRY_DAYS).contains(&days) {
        errors.push(format!(
            "Expiry must be between 1 and {MAX_EXPIRY_DAYS} days."
        ));
    }

    if valid_email {
        match sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM users WHERE lower(email) = $1)",
        )
        .bind(&email)
        .fetch_one(&state.db)
        .await
        {
            Ok(true) => errors.push(
                "An account with that email already exists. Sign in and add them as a member instead."
                    .to_owned(),
            ),
            Ok(false) => {}
            Err(err) => {
                tracing::error!(error = %err, "invitation email lookup failed");
                errors.push("The email could not be checked. Try again.".to_owned());
            }
        }
    }

    errors
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct NewValues {
    email: String,
    full_name: String,
    expiry_days: String,
}

impl NewValues {
    fn from_form(form: &HashMap<String, String>) -> Self {
        Self {
            email: form.get("email").cloned().unwrap_or_default(),
            full_name: form.get("full_name").cloned().unwrap_or_default(),
            expiry_days: form
                .get("expiry_days")
                .cloned()
                .unwrap_or_else(|| "7".to_owned()),
        }
    }

    fn expiry_days(&self) -> i64 {
        self.expiry_days.trim().parse().unwrap_or(7)
    }
}

struct NewPage<'a> {
    session: &'a AdminSession,
    csrf: &'a str,
    sets: &'a [SetOption],
    values: NewValues,
    errors: Vec<String>,
}

fn render_new(page: NewPage<'_>) -> String {
    let mut body = String::new();
    body.push_str(&html::page_head(
        "Invite a user",
        "Create a one-time link. The link is shown once and grants the selected permission sets.",
    ));
    body.push_str("<div class=\"notice\"><strong>Treat the link as a secret.</strong> Anyone who opens it can join with the selected access until it expires or is revoked. Send it over a channel you trust, and only grant capabilities you hold yourself.</div>");

    if !page.errors.is_empty() {
        body.push_str("<div class=\"alert\"><strong>Please fix the following:</strong><ul>");
        for error in &page.errors {
            body.push_str(&format!("<li>{}</li>", html::escape(error)));
        }
        body.push_str("</ul></div>");
    }

    body.push_str(&format!(
        "<form class=\"form wide\" method=\"post\" action=\"/admin/invitations\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        html::escape(page.csrf)
    ));

    body.push_str(&format!(
        "<label for=\"email\">Email</label><input id=\"email\" name=\"email\" type=\"email\" value=\"{}\" required>",
        html::escape(&page.values.email)
    ));
    body.push_str(&format!(
        "<label for=\"full_name\">Full name <span class=\"muted\">(optional)</span></label>\
         <input id=\"full_name\" name=\"full_name\" value=\"{}\">",
        html::escape(&page.values.full_name)
    ));
    body.push_str(&format!(
        "<label for=\"expiry_days\">Expires in (days)</label>\
         <input id=\"expiry_days\" name=\"expiry_days\" type=\"number\" min=\"1\" max=\"{MAX_EXPIRY_DAYS}\" value=\"{}\">",
        html::escape(&page.values.expiry_days)
    ));

    body.push_str("<h2>Permission sets</h2>");
    if page.sets.is_empty() {
        body.push_str("<p class=\"sub\">You do not have any permission sets that you can grant. Ask an owner to invite this person.</p>");
    } else {
        for set in page.sets {
            body.push_str(&format!(
                "<div class=\"checkbox\"><input id=\"set_{id}\" name=\"set_{id}\" type=\"checkbox\">\
                 <label for=\"set_{id}\"><strong>{name}</strong>{desc}</label></div>",
                id = set.id,
                name = html::escape(&set.name),
                desc = set
                    .description
                    .as_deref()
                    .filter(|text| !text.is_empty())
                    .map(|text| format!(
                        "<br><span class=\"muted\">{}</span>",
                        html::escape(text)
                    ))
                    .unwrap_or_default(),
            ));
        }
    }

    body.push_str("<button type=\"submit\">Create invitation link</button></form>");

    html::layout(
        "Invite a user",
        "/admin/invitations",
        Some(&page.session.who()),
        &body,
    )
}

fn link_page(
    session: &AdminSession,
    email: &str,
    full_name: &str,
    selected: &[Uuid],
    sets: &[SetOption],
    link: &str,
) -> String {
    let names = sets
        .iter()
        .filter(|set| selected.contains(&set.id))
        .map(|set| set.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let body = format!(
        "<div class=\"page-head\"><div><h1>Invitation created</h1>\
         <p class=\"muted\">Send this link to {} — it will not be shown again.</p></div></div>\
         <div class=\"warn-box\"><strong>Copy the link now.</strong> \
         Only a hash is stored, so the link cannot be recovered. If it is lost, revoke the invitation and create a new one.</div>\
         <p><input value=\"{}\" readonly style=\"width:100%; padding:10px; font-family:monospace\" onclick=\"this.select()\"></p>\
         <p class=\"sub\">Account: {}{} · Access: {}</p>\
         <p><a class=\"button\" href=\"/admin/invitations\">Back to invitations</a></p>",
        html::escape(email),
        html::escape(link),
        html::escape(email),
        if full_name.is_empty() {
            String::new()
        } else {
            format!(" ({})", html::escape(full_name))
        },
        html::escape(&names),
    );

    html::layout(
        "Invitation created",
        "/admin/invitations",
        Some(&session.who()),
        &body,
    )
}

fn accept_page(invitation: &AcceptInvitation, token: &str, message: Option<&str>) -> String {
    let alert = message.map_or_else(String::new, |message| {
        format!("<div class=\"alert\">{}</div>", html::escape(message))
    });

    let body = format!(
        "<div class=\"page-head\"><div><h1>Join {}</h1>\
         <p class=\"muted\">You have been invited as {}. Access: {}</p></div></div>\
         <form class=\"form\" method=\"post\" action=\"/invite/{}\">\
         {alert}\
         <label for=\"email\">Email</label><input id=\"email\" value=\"{}\" disabled>\
         <label for=\"password\">Password</label><input id=\"password\" name=\"password\" type=\"password\" autocomplete=\"new-password\" required>\
         <label for=\"confirmation\">Confirm password</label><input id=\"confirmation\" name=\"confirmation\" type=\"password\" autocomplete=\"new-password\" required>\
         <button type=\"submit\">Create account and sign in</button></form>\
         <p class=\"hint\">At least 12 characters. Existing accounts cannot accept an invitation — sign in and ask an administrator to add you.</p>",
        html::escape(&invitation.organisation_name),
        html::escape(&invitation.email),
        html::escape(&invitation.permission_sets),
        html::escape(token),
        html::escape(&invitation.email),
    );

    html::layout("Accept invitation", "", None, &body)
}

fn invalid_page() -> String {
    html::layout(
        "Invitation",
        "",
        None,
        "<div class=\"page-head\"><div><h1>Invitation not valid</h1>\
         <p class=\"muted\">This invitation is not valid, has expired, or has already been used.</p></div></div>\
         <p>Ask the person who invited you for a new link.</p>",
    )
}

fn invite_headers() -> [(header::HeaderName, &'static str); 2] {
    [
        (header::CACHE_CONTROL, "no-store"),
        (header::REFERRER_POLICY, "no-referrer"),
    ]
}

fn trim_optional(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.code().as_deref() == Some("23505"))
}

fn failure(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page("Invitations", message)),
    )
        .into_response()
}

fn csrf_failure() -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(html::error_page(
            "Request expired",
            "Reload the page and submit the form again.",
        )),
    )
        .into_response()
}
