//! Admin screens for intake flows, stages and policy.

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    admin::{
        auth::{write_audit, AdminSession},
        csrf, html,
    },
    api::intake::{clone_flow_stages, load_flows, validate_fields, validate_key, validate_name},
    api::intake_cases::admin_board,
    intake::{self, builtin_fields, ACTION_KEYS, STAGE_KINDS},
    permissions::Capability,
    state::AppState,
};

const ACTIVE: &str = "/admin/intake";

#[derive(Debug, FromRow)]
struct CustomFieldRow {
    key: String,
    label: String,
}

#[derive(Debug, FromRow)]
struct UserRow {
    id: Uuid,
    full_name: String,
    email: String,
}

#[derive(Debug, FromRow)]
struct PoolRow {
    id: Uuid,
    name: String,
}

#[derive(Clone)]
struct FieldOption {
    key: String,
    label: String,
    section: String,
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

pub async fn list(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    list_page(&state, &session, jar, None).await
}

async fn list_page(
    state: &AppState,
    session: &AdminSession,
    jar: CookieJar,
    error: Option<String>,
) -> Response {
    let flows = match load_flows(&state.db, session.organisation_id, true).await {
        Ok(flows) => flows,
        Err(err) => {
            tracing::error!(error = %err, "intake flow list failed");
            return failure("Intake flows could not be loaded.");
        }
    };
    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());

    let mut body = String::new();
    body.push_str(&html::page_head(
        "Intake",
        "Flows, stages, fields, checklists and stage owners.",
    ));
    body.push_str(
        "<div class=\"notice\"><strong>Stages are the board columns.</strong> \
         Contact lifecycle statuses (enquiry, onboarding, active, suspended, offboarding, \
         cancelled, offboarded) are separate and are not configured here.</div>",
    );
    if let Some(error) = error {
        body.push_str(&format!(
            "<div class=\"warn-box\">{}</div>",
            html::escape(&error)
        ));
    }

    body.push_str("<p><a class=\"button\" href=\"/admin/intake/board\">Open board</a></p>");
    body.push_str("<h2>Create a flow</h2>");
    body.push_str(&format!(
        "<form method=\"post\" action=\"/admin/intake\" class=\"form\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <div class=\"grid\">\
         <label>Key<input name=\"key\" required placeholder=\"client_aged_care_v2\"></label>\
         <label>Name<input name=\"name\" required placeholder=\"Client — standard\"></label>\
         <label>Subject kind<select name=\"subject_kind\">{}</select></label>\
         <label>Variant<input name=\"variant\" placeholder=\"standard\"></label>\
         <label>Description<textarea name=\"description\" rows=\"2\" placeholder=\"What is this path for?\"></textarea></label>\
         <label>Stage movement<select name=\"stage_movement\"><option value=\"any\">Any stage</option><option value=\"sequential\" selected>Sequential</option></select></label>\
         <label>Parent flow<select name=\"parent_flow_id\"><option value=\"\">None</option>{}</select></label>\
         <label>Clone stages from<select name=\"clone_from_flow_id\"><option value=\"\">Same subject kind default</option>{}</select></label>\
         </div>\
         <label class=\"check\"><input type=\"checkbox\" name=\"is_default\" value=\"1\"> Default flow for this subject kind and variant</label>\
         <button type=\"submit\">Create flow</button>\
         </form>",
        html::escape(&token),
        subject_kind_options(),
        flow_options(&flows, None),
        flow_options(&flows, None),
    ));

    if flows.is_empty() {
        body.push_str("<p class=\"sub\">No intake flows yet.</p>");
        return page(session, body);
    }

    body.push_str("<h2>Flows</h2>");
    for flow in &flows {
        body.push_str(&format!(
            "<div class=\"card\"><div class=\"card-head\"><div><strong>{}</strong> \
             <code>{}</code> <span class=\"badge {}\">{}</span> \
             <span class=\"muted\">{} / {} / {}</span></div>\
             <a class=\"button\" href=\"/admin/intake/flows/{}\">Open</a></div>",
            html::escape(&flow.name),
            html::escape(&flow.key),
            if flow.is_active { "pk" } else { "id" },
            if flow.is_active { "active" } else { "archived" },
            html::escape(&flow.subject_kind),
            html::escape(flow.variant.as_deref().unwrap_or("—")),
            html::escape(&flow.stage_movement),
            flow.id,
        ));
        if let Some(parent) = flow.parent_flow_id {
            body.push_str(&format!(
                "<p class=\"sub\">Sub-flow of {}</p>",
                html::escape(
                    flows
                        .iter()
                        .find(|candidate| candidate.id == parent)
                        .map_or("another flow", |candidate| candidate.name.as_str())
                )
            ));
        }
        if let Some(description) = flow
            .description
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            body.push_str(&format!(
                "<p class=\"sub\">{}</p>",
                html::escape(description)
            ));
        }
        body.push_str(
            "<table><thead><tr><th>#</th><th>Stage</th><th>Kind</th><th>Fields</th>\
             <th>Checklist</th><th>Owner</th><th>Actions</th></tr></thead><tbody>",
        );
        for stage in &flow.stages {
            let owner = stage
                .assignees
                .first()
                .and_then(|rule| {
                    rule.user
                        .as_ref()
                        .map(|user| user.full_name.clone())
                        .or_else(|| {
                            rule.permission_set
                                .as_ref()
                                .map(|set| format!("{} pool", set.name))
                        })
                })
                .unwrap_or_else(|| "—".to_owned());
            let allowed: Vec<&str> = stage
                .actions
                .iter()
                .filter(|(_, allowed)| **allowed)
                .map(|(key, _)| key.as_str())
                .collect();
            body.push_str(&format!(
                "<tr><td>{}</td><td><a href=\"/admin/intake/stages/{}\">{}</a></td>\
                 <td><code>{}</code></td><td>{} visible</td><td>{}</td><td>{}</td>\
                 <td>{}</td></tr>",
                stage.sort_order,
                stage.id,
                html::escape(&stage.name),
                html::escape(&stage.kind),
                stage.visible_fields.len(),
                stage.checklist.len(),
                html::escape(&owner),
                if allowed.is_empty() {
                    "<span class=\"muted\">none</span>".to_owned()
                } else {
                    allowed
                        .iter()
                        .map(|key| format!("<span class=\"badge id\">{}</span>", html::escape(key)))
                        .collect::<Vec<_>>()
                        .join(" ")
                },
            ));
        }
        body.push_str("</tbody></table></div>");
    }

    (jar, Html(render(session, body))).into_response()
}

#[derive(Debug, Default, Deserialize)]
pub struct BoardParams {
    flow_id: Option<Uuid>,
}

pub async fn board_page(
    State(state): State<AppState>,
    session: AdminSession,
    Query(params): Query<BoardParams>,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    let flows = match load_flows(&state.db, session.organisation_id, true).await {
        Ok(flows) => flows,
        Err(err) => {
            tracing::error!(error = %err, "intake board flow load failed");
            return failure("The intake board could not be loaded.");
        }
    };
    let Some(flow) = params
        .flow_id
        .and_then(|id| flows.iter().find(|flow| flow.id == id))
        .or_else(|| flows.iter().find(|flow| flow.is_active))
        .or_else(|| flows.first())
    else {
        return failure("There are no intake flows yet.");
    };
    let board = match admin_board(&state.db, session.organisation_id, flow.id, 50).await {
        Ok(board) => board,
        Err(err) => {
            tracing::error!(error = %err, "intake board load failed");
            return failure("The intake board could not be loaded.");
        }
    };

    let mut body = String::new();
    body.push_str(&format!(
        "<p><a href=\"/admin/intake\">← Intake configuration</a></p>\
         <div class=\"page-head\"><div><h1>{}</h1><p class=\"muted\">Read-only board. The desktop client will become the interactive kanban.</p></div></div>\
         <form method=\"get\" action=\"/admin/intake/board\" class=\"form\">\
         <label>Flow<select name=\"flow_id\" onchange=\"this.form.submit()\">{}</select></label>\
         </form>",
        html::escape(&flow.name),
        flows
            .iter()
            .map(|candidate| format!(
                "<option value=\"{}\" {}>{} ({})</option>",
                candidate.id,
                if candidate.id == flow.id { "selected" } else { "" },
                html::escape(&candidate.name),
                html::escape(&candidate.subject_kind),
            ))
            .collect::<String>(),
    ));

    body.push_str("<div style=\"display:flex;gap:0.9rem;overflow-x:auto;padding-bottom:1rem;align-items:flex-start\">");
    for column in &board.columns {
        body.push_str(&format!(
            "<div style=\"flex:0 0 260px;background:var(--card);border:1px solid var(--border);border-radius:10px;padding:0.75rem\">\
             <div style=\"display:flex;justify-content:space-between;align-items:center;margin-bottom:0.6rem\">\
             <strong>{}</strong><span class=\"badge pk\">{}</span></div>",
            html::escape(&column.stage.name),
            column.count,
        ));
        if column.cards.is_empty() {
            body.push_str("<p class=\"sub\">No cases.</p>");
        }
        for card in &column.cards {
            let lifecycle = card
                .lifecycle_status
                .as_deref()
                .map(|status| format!("<span class=\"badge id\">{status}</span>"))
                .unwrap_or_default();
            let division = card
                .division
                .as_ref()
                .map(|division| format!(" · {}", html::escape(&division.name)))
                .unwrap_or_default();
            let assignee = card
                .assignee
                .as_ref()
                .map(|user| format!(" · {}", html::escape(&user.full_name)))
                .unwrap_or_else(|| " · unassigned".to_owned());
            let waitlist = if card.waitlisted {
                " · <span class=\"badge yes\">waitlist</span>"
            } else {
                ""
            };
            let due = card
                .stage_due_at
                .map(|due| format!(" · due {}", due.format("%d %b")))
                .unwrap_or_default();
            body.push_str(&format!(
                "<div class=\"card\" style=\"margin:0 0 0.6rem 0;padding:0.7rem\">\
                 <div><strong>{}</strong></div>\
                 <div class=\"sub\" style=\"margin-top:0.25rem\">{}{}{}{}{}</div>\
                 <div class=\"sub\" style=\"margin-top:0.35rem\">checklist {}/{}</div>\
                 </div>",
                html::escape(&card.display_name),
                lifecycle,
                division,
                assignee,
                waitlist,
                due,
                card.checklist.done,
                card.checklist.total,
            ));
        }
        body.push_str("</div>");
    }
    body.push_str("</div>");

    (Html(render(&session, body))).into_response()
}

// ---------------------------------------------------------------------------
// Flow page
// ---------------------------------------------------------------------------

pub async fn flow_page(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    let flows = match load_flows(&state.db, session.organisation_id, true).await {
        Ok(flows) => flows,
        Err(err) => {
            tracing::error!(error = %err, "intake flow load failed");
            return failure("The intake flow could not be loaded.");
        }
    };
    let Some(flow) = flows.iter().find(|flow| flow.id == id) else {
        return not_found();
    };
    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());

    let mut body = String::new();
    body.push_str(&format!(
        "<p><a href=\"/admin/intake\">← All intake flows</a></p>\
         <div class=\"page-head\"><div><h1>{}</h1><p class=\"muted\">{} · {} · {}</p></div></div>",
        html::escape(&flow.name),
        html::escape(&flow.key),
        html::escape(&flow.subject_kind),
        html::escape(flow.variant.as_deref().unwrap_or("no variant")),
    ));

    body.push_str(&format!(
        "<p><a class=\"button\" href=\"/admin/intake/board?flow_id={}\">Open board</a></p>",
        flow.id
    ));
    body.push_str("<h2>Flow settings</h2>");
    body.push_str(&format!(
        "<form method=\"post\" action=\"/admin/intake/flows/{}/update\" class=\"form\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <div class=\"grid\">\
         <label>Name<input name=\"name\" required value=\"{}\"></label>\
         <label>Variant<input name=\"variant\" value=\"{}\"></label>\
         <label>Description<textarea name=\"description\" rows=\"2\">{}</textarea></label>\
         <label>Stage movement<select name=\"stage_movement\">{}</select></label>\
         </div>\
         <label class=\"check\"><input type=\"checkbox\" name=\"is_active\" value=\"1\" {}> Active</label>\
         <label class=\"check\"><input type=\"checkbox\" name=\"is_default\" value=\"1\" {}> Default for this subject kind and variant</label>\
         <button type=\"submit\">Save flow</button>\
         </form>",
        flow.id,
        html::escape(&token),
        html::escape(&flow.name),
        html::escape(flow.variant.as_deref().unwrap_or("")),
        html::escape(flow.description.as_deref().unwrap_or("")),
        stage_movement_options(Some(&flow.stage_movement)),
        if flow.is_active { "checked" } else { "" },
        if flow.is_default { "checked" } else { "" },
    ));

    body.push_str("<h2>Stages</h2>");
    body.push_str(
        "<p class=\"sub\">The first stage must be Enquiry. Onboarded and Cancelled are the \
         terminal stages. Reorder stages through the configuration API.</p>",
    );
    body.push_str(
        "<table><thead><tr><th>#</th><th>Stage</th><th>Kind</th><th>Terminal</th>\
         <th>Nudge</th><th>Fields</th><th>Owner</th><th></th></tr></thead><tbody>",
    );
    for stage in &flow.stages {
        let owner = stage
            .assignees
            .first()
            .and_then(|rule| {
                rule.user
                    .as_ref()
                    .map(|user| user.full_name.clone())
                    .or_else(|| {
                        rule.permission_set
                            .as_ref()
                            .map(|set| format!("{} pool", set.name))
                    })
            })
            .unwrap_or_else(|| "unassigned".to_owned());
        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td>\
             <td>{}</td><td>{} visible / {} required</td><td>{}</td>\
             <td><a href=\"/admin/intake/stages/{}\">Edit</a></td></tr>",
            stage.sort_order,
            html::escape(&stage.name),
            html::escape(&stage.kind),
            if stage.is_terminal { "yes" } else { "no" },
            html::escape(stage.on_enter_contact_status.as_deref().unwrap_or("—")),
            stage.visible_fields.len(),
            stage.required_fields.len(),
            html::escape(&owner),
            stage.id,
        ));
    }
    body.push_str("</tbody></table>");

    body.push_str("<h2>Add a stage</h2>");
    body.push_str(&format!(
        "<form method=\"post\" action=\"/admin/intake/flows/{}/stages\" class=\"form\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <div class=\"grid\">\
         <label>Key<input name=\"key\" required placeholder=\"funding_check\"></label>\
         <label>Name<input name=\"name\" required placeholder=\"Funding check\"></label>\
         <label>Kind<select name=\"kind\">{}</select></label>\
         <label>Colour<input name=\"color\" placeholder=\"blue\"></label>\
         <label>SLA hours<input name=\"sla_hours\" type=\"number\" min=\"1\" placeholder=\"48\"></label>\
         </div>\
         <label class=\"check\"><input type=\"checkbox\" name=\"checklist_gate\" value=\"1\"> Required checklist must be complete before leaving</label>\
         <button type=\"submit\">Add stage</button>\
         </form>",
        flow.id,
        html::escape(&token),
        stage_kind_options(None),
    ));

    body.push_str(&format!(
        "<h2>Archive</h2>\
         <form method=\"post\" action=\"/admin/intake/flows/{}/archive\" class=\"form\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <p class=\"sub\">Archiving hides the flow from the picker but keeps its cases readable.</p>\
         <button class=\"danger\" type=\"submit\">Archive flow</button>\
         </form>",
        flow.id,
        html::escape(&token),
    ));

    (jar, Html(render(&session, body))).into_response()
}

// ---------------------------------------------------------------------------
// Stage page
// ---------------------------------------------------------------------------

pub async fn stage_page(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    let flows = match load_flows(&state.db, session.organisation_id, true).await {
        Ok(flows) => flows,
        Err(err) => {
            tracing::error!(error = %err, "intake stage load failed");
            return failure("The intake stage could not be loaded.");
        }
    };
    let Some((flow, stage)) = find_stage(&flows, id) else {
        return not_found();
    };

    let fields = match field_options(&state, session.organisation_id).await {
        Ok(fields) => fields,
        Err(err) => {
            tracing::error!(error = %err, "intake field catalogue failed");
            return failure("The field catalogue could not be loaded.");
        }
    };
    let users = users(&state, session.organisation_id)
        .await
        .unwrap_or_default();
    let pools = pools(&state, session.organisation_id)
        .await
        .unwrap_or_default();
    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());

    let mut body = String::new();
    body.push_str(&format!(
        "<p><a href=\"/admin/intake/flows/{}\">← {}</a></p>\
         <div class=\"page-head\"><div><h1>{}</h1><p class=\"muted\">{} · {} · {}</p></div></div>",
        flow.id,
        html::escape(&flow.name),
        html::escape(&stage.name),
        html::escape(&flow.name),
        html::escape(&stage.key),
        html::escape(&stage.kind),
    ));

    body.push_str("<h2>Stage settings</h2>");
    body.push_str(&format!(
        "<form method=\"post\" action=\"/admin/intake/stages/{}/update\" class=\"form\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <div class=\"grid\">\
         <label>Name<input name=\"name\" required value=\"{}\"></label>\
         <label>Kind<input value=\"{}\" disabled></label>\
         <label>Colour<input name=\"color\" value=\"{}\"></label>\
         <label>Lifecycle nudge<select name=\"on_enter_contact_status\"><option value=\"\">None</option>{}</select></label>\
         <label>SLA hours<input name=\"sla_hours\" type=\"number\" min=\"1\" value=\"{}\"></label>\
         </div>\
         <label class=\"check\"><input type=\"checkbox\" name=\"checklist_gate\" value=\"1\" {}> Required checklist must be complete before leaving</label>\
         <button type=\"submit\">Save stage</button>\
         </form>",
        stage.id,
        html::escape(&token),
        html::escape(&stage.name),
        html::escape(&stage.kind),
        html::escape(stage.color.as_deref().unwrap_or("")),
        lifecycle_options(stage.on_enter_contact_status.as_deref()),
        stage.sla_hours.map_or(String::new(), |hours| hours.to_string()),
        if stage.checklist_gate { "checked" } else { "" },
    ));

    body.push_str("<h2>Visible and required fields</h2>");
    body.push_str(&format!(
        "<form method=\"post\" action=\"/admin/intake/stages/{}/fields\" class=\"form\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <p class=\"sub\">Tick the fields shown in this stage. Required fields must also be visible.</p>",
        stage.id,
        html::escape(&token),
    ));
    let mut sections: BTreeMap<&str, Vec<&FieldOption>> = BTreeMap::new();
    for field in &fields {
        sections
            .entry(field.section.as_str())
            .or_default()
            .push(field);
    }
    for (section, fields) in sections {
        body.push_str(&format!(
            "<fieldset><legend>{}</legend><table><thead><tr><th>Field</th><th>Visible</th><th>Required</th></tr></thead><tbody>",
            html::escape(section)
        ));
        for field in fields {
            let visible = stage.visible_fields.contains(&field.key);
            let required = stage.required_fields.contains(&field.key);
            body.push_str(&format!(
                "<tr><td>{} <code>{}</code></td>\
                 <td><input type=\"checkbox\" name=\"visible__{}\" value=\"1\" {}</td>\
                 <td><input type=\"checkbox\" name=\"required__{}\" value=\"1\" {}</td></tr>",
                html::escape(&field.label),
                html::escape(&field.key),
                html::escape(&field.key),
                if visible { "checked" } else { "" },
                html::escape(&field.key),
                if required { "checked" } else { "" },
            ));
        }
        body.push_str("</tbody></table></fieldset>");
    }
    body.push_str("<button type=\"submit\">Save fields</button></form>");

    body.push_str("<h2>Allowed actions</h2>");
    body.push_str(&format!(
        "<form method=\"post\" action=\"/admin/intake/stages/{}/actions\" class=\"form\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <p class=\"sub\">Stage policy can deny an action the user's capability would otherwise allow. It never grants an action on its own.</p>\
         <div class=\"checks\">",
        stage.id,
        html::escape(&token),
    ));
    for action in ACTION_KEYS {
        body.push_str(&format!(
            "<label class=\"check\"><input type=\"checkbox\" name=\"action__{}\" value=\"1\" {}> <code>{}</code></label>",
            html::escape(action),
            if stage.actions.get(action).copied().unwrap_or(false) {
                "checked"
            } else {
                ""
            },
            html::escape(action),
        ));
    }
    body.push_str("</div><button type=\"submit\">Save actions</button></form>");

    body.push_str("<h2>Checklist</h2>");
    body.push_str(&format!(
        "<form method=\"post\" action=\"/admin/intake/stages/{}/checklist\" class=\"form\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <p class=\"sub\">One item per line. Prefix <code>*</code> for required or <code>-</code> for optional. Use <code>|</code> to add a description.</p>\
         <textarea name=\"items\" rows=\"8\" class=\"wide\">{}</textarea>\
         <button type=\"submit\">Save checklist</button>\
         </form>",
        stage.id,
        html::escape(&token),
        html::escape(&checklist_text(stage)),
    ));

    body.push_str("<h2>Who handles this stage?</h2>");
    body.push_str(&format!(
        "<form method=\"post\" action=\"/admin/intake/stages/{}/assignees\" class=\"form\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <p class=\"sub\">The first configured rule sets the primary assignee. Additional rules receive the alert as backup or watchers. Leave both target fields blank to remove a rule.</p>",
        stage.id,
        html::escape(&token),
    ));
    for index in 0..4 {
        let rule = stage.assignees.get(index);
        let user_id = rule
            .and_then(|rule| rule.user.as_ref().map(|user| user.id))
            .map(|id| id.to_string())
            .unwrap_or_default();
        let pool_id = rule
            .and_then(|rule| rule.permission_set.as_ref().map(|set| set.id))
            .map(|id| id.to_string())
            .unwrap_or_default();
        let strategy = rule.map_or("fixed", |rule| rule.strategy.as_str());
        body.push_str(&format!(
            "<fieldset><legend>Rule {}</legend><div class=\"grid\">\
             <label>User<select name=\"rule_user_{index}\"><option value=\"\">—</option>{}</select></label>\
             <label>Permission set<select name=\"rule_pool_{index}\"><option value=\"\">—</option>{}</select></label>\
             <label>Strategy<select name=\"rule_strategy_{index}\">{}</select></label>\
             </div>\
             <label class=\"check\"><input type=\"checkbox\" name=\"rule_notify_{index}\" value=\"1\" {}> Notify this rule</label>\
             </fieldset>",
            index + 1,
            user_options(&users, &user_id),
            pool_options(&pools, &pool_id),
            strategy_options(strategy),
            if rule.is_none_or(|rule| rule.notify) {
                "checked"
            } else {
                ""
            },
        ));
    }
    body.push_str("<button type=\"submit\">Save assignment</button></form>");

    if !stage.is_terminal {
        body.push_str(&format!(
            "<h2>Delete stage</h2>\
             <form method=\"post\" action=\"/admin/intake/stages/{}/delete\" class=\"form\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <p class=\"sub\">A stage with cases, or the first stage, cannot be deleted.</p>\
             <button class=\"danger\" type=\"submit\">Delete stage</button>\
             </form>",
            stage.id,
            html::escape(&token),
        ));
    }

    (jar, Html(render(&session, body))).into_response()
}

// ---------------------------------------------------------------------------
// Flow writes
// ---------------------------------------------------------------------------

pub async fn create_flow(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    if !verify(&jar, &form) {
        return csrf_failure();
    }
    let result = create_flow_inner(&state, &session, &form).await;
    match result {
        Ok(id) => Redirect::to(&format!("/admin/intake/flows/{id}")).into_response(),
        Err(message) => list_page(&state, &session, jar, Some(message)).await,
    }
}

async fn create_flow_inner(
    state: &AppState,
    session: &AdminSession,
    form: &HashMap<String, String>,
) -> Result<Uuid, String> {
    let key = text(form, "key");
    validate_key(&key).map_err(|err| err.to_string())?;
    let name = validate_name(&text(form, "name")).map_err(|err| err.to_string())?;
    let subject_kind = text(form, "subject_kind");
    if !intake::is_subject_kind(&subject_kind) {
        return Err("Choose a valid subject kind.".to_owned());
    }
    let parent_flow_id = parse_uuid(form.get("parent_flow_id"));
    let clone_from_flow_id = parse_uuid(form.get("clone_from_flow_id"));
    let variant = optional_text(form, "variant");
    let description = optional_text(form, "description");
    let stage_movement = optional_text(form, "stage_movement").unwrap_or_else(|| {
        if subject_kind == "client" {
            "sequential".to_owned()
        } else {
            "any".to_owned()
        }
    });
    if !matches!(stage_movement.as_str(), "any" | "sequential") {
        return Err("Choose a valid stage movement.".to_owned());
    }
    let is_default = form.contains_key("is_default");

    let mut tx = state.db.begin().await.map_err(|err| err.to_string())?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO intake_flows
            (id, organisation_id, parent_flow_id, key, name, subject_kind, variant,
             description, is_default, is_active, stage_movement)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, true, $10)",
    )
    .bind(id)
    .bind(session.organisation_id)
    .bind(parent_flow_id)
    .bind(&key)
    .bind(&name)
    .bind(&subject_kind)
    .bind(&variant)
    .bind(&description)
    .bind(is_default)
    .bind(&stage_movement)
    .execute(&mut *tx)
    .await
    .map_err(friendly_db_error)?;

    if is_default {
        sqlx::query(
            "UPDATE intake_flows SET is_default = false
             WHERE organisation_id = $1 AND subject_kind = $2 AND id <> $3
               AND COALESCE(variant, '') = COALESCE($4, '')",
        )
        .bind(session.organisation_id)
        .bind(&subject_kind)
        .bind(id)
        .bind(&variant)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    }

    let source = if let Some(source) = clone_from_flow_id {
        Some(source)
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM intake_flows
             WHERE organisation_id = $1 AND subject_kind = $2 AND is_active
               AND id <> $3
             ORDER BY sort_order, id LIMIT 1",
        )
        .bind(session.organisation_id)
        .bind(&subject_kind)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| err.to_string())?
    };
    if let Some(source) = source {
        if source != id {
            clone_flow_stages(&mut tx, session.organisation_id, source, id, &subject_kind)
                .await
                .map_err(|err| err.to_string())?;
        }
    }

    tx.commit().await.map_err(|err| err.to_string())?;
    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "intake.flow.create",
        "intake_flow",
        Some(id),
    )
    .await;
    Ok(id)
}

pub async fn update_flow(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    if !verify(&jar, &form) {
        return csrf_failure();
    }
    let name = match validate_name(&text(&form, "name")) {
        Ok(name) => name,
        Err(err) => return failure(&err.to_string()),
    };
    let variant = optional_text(&form, "variant");
    let description = optional_text(&form, "description");
    let stage_movement = optional_text(&form, "stage_movement").unwrap_or_else(|| "any".to_owned());
    if !matches!(stage_movement.as_str(), "any" | "sequential") {
        return failure("Choose a valid stage movement.");
    }
    let is_active = form.contains_key("is_active");
    let is_default = form.contains_key("is_default");

    let result = sqlx::query(
        "UPDATE intake_flows
         SET name = $1, variant = $2, description = $3, is_active = $4,
             is_default = $5, stage_movement = $6
         WHERE organisation_id = $7 AND id = $8",
    )
    .bind(&name)
    .bind(&variant)
    .bind(&description)
    .bind(is_active)
    .bind(is_default)
    .bind(&stage_movement)
    .bind(session.organisation_id)
    .bind(id)
    .execute(&state.db)
    .await;
    if let Err(err) = result {
        tracing::error!(error = %err, "intake flow update failed");
        return failure("The flow could not be saved.");
    }
    if is_default {
        let _ = sqlx::query(
            "UPDATE intake_flows SET is_default = false
             WHERE organisation_id = $1 AND subject_kind = (
                 SELECT subject_kind FROM intake_flows WHERE id = $2
             ) AND id <> $2",
        )
        .bind(session.organisation_id)
        .bind(id)
        .execute(&state.db)
        .await;
    }
    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "intake.flow.update",
        "intake_flow",
        Some(id),
    )
    .await;
    Redirect::to(&format!("/admin/intake/flows/{id}")).into_response()
}

pub async fn archive_flow(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    if !verify(&jar, &form) {
        return csrf_failure();
    }
    let _ = sqlx::query(
        "UPDATE intake_flows SET is_active = false
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(session.organisation_id)
    .bind(id)
    .execute(&state.db)
    .await;
    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "intake.flow.archive",
        "intake_flow",
        Some(id),
    )
    .await;
    Redirect::to(ACTIVE).into_response()
}

pub async fn add_stage(
    State(state): State<AppState>,
    session: AdminSession,
    Path(flow_id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    if !verify(&jar, &form) {
        return csrf_failure();
    }
    let result = add_stage_inner(&state, &session, flow_id, &form).await;
    match result {
        Ok(stage_id) => Redirect::to(&format!("/admin/intake/stages/{stage_id}")).into_response(),
        Err(message) => failure(&message),
    }
}

async fn add_stage_inner(
    state: &AppState,
    session: &AdminSession,
    flow_id: Uuid,
    form: &HashMap<String, String>,
) -> Result<Uuid, String> {
    let key = text(form, "key");
    validate_key(&key).map_err(|err| err.to_string())?;
    let name = validate_name(&text(form, "name")).map_err(|err| err.to_string())?;
    let kind = text(form, "kind");
    if !intake::is_stage_kind(&kind) {
        return Err("Choose a valid stage kind.".to_owned());
    }
    let sla_hours = parse_positive_i32(form.get("sla_hours"))?;
    let checklist_gate = form.contains_key("checklist_gate");
    let color = optional_text(form, "color");

    let flows = load_flows(&state.db, session.organisation_id, true)
        .await
        .map_err(|err| err.to_string())?;
    let flow = flows
        .iter()
        .find(|flow| flow.id == flow_id)
        .ok_or_else(|| "The intake flow was not found.".to_owned())?;
    if flow.stages.iter().any(|stage| stage.key == key) {
        return Err("That stage key is already used in this flow.".to_owned());
    }
    if matches!(kind.as_str(), "enquiry" | "onboarded" | "cancelled")
        && flow.stages.iter().any(|stage| stage.kind == kind)
    {
        return Err(format!("This flow already has a {kind} stage."));
    }

    let max_order = flow
        .stages
        .iter()
        .map(|stage| stage.sort_order)
        .max()
        .unwrap_or(0)
        .saturating_add(10);
    let (sort_order, shift_from) = if intake::is_terminal_kind(&kind) {
        (max_order, None)
    } else if let Some(first_terminal) = flow.stages.iter().find(|stage| stage.is_terminal) {
        (first_terminal.sort_order, Some(first_terminal.sort_order))
    } else {
        (max_order, None)
    };

    let mut tx = state.db.begin().await.map_err(|err| err.to_string())?;
    if let Some(threshold) = shift_from {
        sqlx::query(
            "UPDATE intake_stages SET sort_order = sort_order + 10
             WHERE organisation_id = $1 AND flow_id = $2 AND sort_order >= $3",
        )
        .bind(session.organisation_id)
        .bind(flow_id)
        .bind(threshold)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    }

    let stage_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO intake_stages
            (id, organisation_id, flow_id, key, name, stage_kind, color, sort_order,
             is_terminal, on_enter_contact_status, sla_hours, checklist_gate,
             visible_fields, required_fields)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL, $10, $11,
                 '[]'::jsonb, '[]'::jsonb)",
    )
    .bind(stage_id)
    .bind(session.organisation_id)
    .bind(flow_id)
    .bind(&key)
    .bind(&name)
    .bind(&kind)
    .bind(&color)
    .bind(sort_order)
    .bind(intake::is_terminal_kind(&kind))
    .bind(sla_hours)
    .bind(checklist_gate)
    .execute(&mut *tx)
    .await
    .map_err(friendly_db_error)?;

    for action in ACTION_KEYS {
        sqlx::query(
            "INSERT INTO intake_stage_actions
                (organisation_id, stage_id, action_key, allowed)
             VALUES ($1, $2, $3, false)",
        )
        .bind(session.organisation_id)
        .bind(stage_id)
        .bind(action)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "intake.stage.create",
        "intake_stage",
        Some(stage_id),
    )
    .await;
    Ok(stage_id)
}

// ---------------------------------------------------------------------------
// Stage writes
// ---------------------------------------------------------------------------

pub async fn update_stage(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    if !verify(&jar, &form) {
        return csrf_failure();
    }
    let name = match validate_name(&text(&form, "name")) {
        Ok(name) => name,
        Err(err) => return failure(&err.to_string()),
    };
    let color = optional_text(&form, "color");
    let nudge = optional_text(&form, "on_enter_contact_status");
    let sla_hours = match parse_positive_i32(form.get("sla_hours")) {
        Ok(value) => value,
        Err(err) => return failure(&err),
    };
    let checklist_gate = form.contains_key("checklist_gate");

    let result = sqlx::query(
        "UPDATE intake_stages
         SET name = $1, color = $2, on_enter_contact_status = $3,
             sla_hours = $4, checklist_gate = $5
         WHERE organisation_id = $6 AND id = $7",
    )
    .bind(&name)
    .bind(&color)
    .bind(&nudge)
    .bind(sla_hours)
    .bind(checklist_gate)
    .bind(session.organisation_id)
    .bind(id)
    .execute(&state.db)
    .await;
    if let Err(err) = result {
        tracing::error!(error = %err, "intake stage update failed");
        return failure("The stage could not be saved.");
    }
    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "intake.stage.update",
        "intake_stage",
        Some(id),
    )
    .await;
    Redirect::to(&format!("/admin/intake/stages/{id}")).into_response()
}

pub async fn update_stage_fields(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    if !verify(&jar, &form) {
        return csrf_failure();
    }
    let mut visible = Vec::new();
    let mut required = Vec::new();
    for key in form.keys() {
        if let Some(field) = key.strip_prefix("visible__") {
            visible.push(field.to_owned());
        } else if let Some(field) = key.strip_prefix("required__") {
            required.push(field.to_owned());
        }
    }
    let custom = custom_keys(&state, session.organisation_id)
        .await
        .unwrap_or_default();
    if let Err(err) = validate_fields(&visible, &required, &custom) {
        return failure(&err.to_string());
    }

    let result = sqlx::query(
        "UPDATE intake_stages SET visible_fields = $1, required_fields = $2
         WHERE organisation_id = $3 AND id = $4",
    )
    .bind(Value::Array(
        visible.into_iter().map(Value::String).collect(),
    ))
    .bind(Value::Array(
        required.into_iter().map(Value::String).collect(),
    ))
    .bind(session.organisation_id)
    .bind(id)
    .execute(&state.db)
    .await;
    if let Err(err) = result {
        tracing::error!(error = %err, "intake stage fields update failed");
        return failure("The stage fields could not be saved.");
    }
    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "intake.stage.fields",
        "intake_stage",
        Some(id),
    )
    .await;
    Redirect::to(&format!("/admin/intake/stages/{id}")).into_response()
}

pub async fn update_stage_actions(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    if !verify(&jar, &form) {
        return csrf_failure();
    }
    let mut allowed = HashSet::new();
    for key in form.keys() {
        if let Some(action) = key.strip_prefix("action__") {
            if !intake::is_action_key(action) {
                return failure("Unknown action key.");
            }
            allowed.insert(action.to_owned());
        }
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, "intake action transaction failed");
            return failure("The actions could not be saved.");
        }
    };
    let _ = sqlx::query(
        "DELETE FROM intake_stage_actions WHERE organisation_id = $1 AND stage_id = $2",
    )
    .bind(session.organisation_id)
    .bind(id)
    .execute(&mut *tx)
    .await;
    for action in ACTION_KEYS {
        let _ = sqlx::query(
            "INSERT INTO intake_stage_actions
                (organisation_id, stage_id, action_key, allowed)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(session.organisation_id)
        .bind(id)
        .bind(action)
        .bind(allowed.contains(action))
        .execute(&mut *tx)
        .await;
    }
    if let Err(err) = tx.commit().await {
        tracing::error!(error = %err, "intake action commit failed");
        return failure("The actions could not be saved.");
    }
    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "intake.stage.actions",
        "intake_stage",
        Some(id),
    )
    .await;
    Redirect::to(&format!("/admin/intake/stages/{id}")).into_response()
}

pub async fn update_stage_checklist(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    if !verify(&jar, &form) {
        return csrf_failure();
    }
    let items = parse_checklist(form.get("items").map(String::as_str).unwrap_or(""));
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, "intake checklist transaction failed");
            return failure("The checklist could not be saved.");
        }
    };
    let _ = sqlx::query(
        "DELETE FROM intake_stage_checklist_items
         WHERE organisation_id = $1 AND stage_id = $2",
    )
    .bind(session.organisation_id)
    .bind(id)
    .execute(&mut *tx)
    .await;
    for (index, item) in items.iter().enumerate() {
        let _ = sqlx::query(
            "INSERT INTO intake_stage_checklist_items
                (id, organisation_id, stage_id, label, description, is_required, sort_order)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::now_v7())
        .bind(session.organisation_id)
        .bind(id)
        .bind(&item.label)
        .bind(&item.description)
        .bind(item.is_required)
        .bind(((index as i32) + 1) * 10)
        .execute(&mut *tx)
        .await;
    }
    if let Err(err) = tx.commit().await {
        tracing::error!(error = %err, "intake checklist commit failed");
        return failure("The checklist could not be saved.");
    }
    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "intake.stage.checklist",
        "intake_stage",
        Some(id),
    )
    .await;
    Redirect::to(&format!("/admin/intake/stages/{id}")).into_response()
}

pub async fn update_stage_assignees(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    if !verify(&jar, &form) {
        return csrf_failure();
    }

    let mut rules = Vec::new();
    for index in 0..4 {
        let user = parse_uuid(form.get(&format!("rule_user_{index}")));
        let pool = parse_uuid(form.get(&format!("rule_pool_{index}")));
        if user.is_none() && pool.is_none() {
            continue;
        }
        if user.is_some() && pool.is_some() {
            return failure("Each assignment rule needs one user or one permission set, not both.");
        }
        let strategy = form
            .get(&format!("rule_strategy_{index}"))
            .map(String::as_str)
            .unwrap_or("fixed");
        let strategy = if user.is_some() {
            "fixed"
        } else if matches!(strategy, "first_available" | "round_robin") {
            strategy
        } else {
            "first_available"
        };
        rules.push((
            user,
            pool,
            strategy.to_owned(),
            form.contains_key(&format!("rule_notify_{index}")),
        ));
    }

    for (user, pool, _, _) in &rules {
        if let Some(user) = *user {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM organisation_users
                 WHERE organisation_id = $1 AND user_id = $2)",
            )
            .bind(session.organisation_id)
            .bind(user)
            .fetch_one(&state.db)
            .await
            .unwrap_or(false);
            if !exists {
                return failure("That user is not a member of this organisation.");
            }
        }
        if let Some(pool) = *pool {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM permission_sets
                 WHERE organisation_id = $1 AND id = $2)",
            )
            .bind(session.organisation_id)
            .bind(pool)
            .fetch_one(&state.db)
            .await
            .unwrap_or(false);
            if !exists {
                return failure("That permission set was not found.");
            }
        }
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, "intake assignment transaction failed");
            return failure("The assignment could not be saved.");
        }
    };
    let _ = sqlx::query(
        "DELETE FROM intake_stage_assignees WHERE organisation_id = $1 AND stage_id = $2",
    )
    .bind(session.organisation_id)
    .bind(id)
    .execute(&mut *tx)
    .await;
    let _ = sqlx::query(
        "DELETE FROM intake_assignment_cursors WHERE organisation_id = $1 AND stage_id = $2",
    )
    .bind(session.organisation_id)
    .bind(id)
    .execute(&mut *tx)
    .await;
    for (index, (user, pool, strategy, notify)) in rules.iter().enumerate() {
        let _ = sqlx::query(
            "INSERT INTO intake_stage_assignees
                (id, organisation_id, stage_id, user_id, permission_set_id,
                 strategy, notify, sort_order)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(Uuid::now_v7())
        .bind(session.organisation_id)
        .bind(id)
        .bind(*user)
        .bind(*pool)
        .bind(strategy.as_str())
        .bind(*notify)
        .bind(((index as i32) + 1) * 10)
        .execute(&mut *tx)
        .await;
    }
    if let Err(err) = tx.commit().await {
        tracing::error!(error = %err, "intake assignment commit failed");
        return failure("The assignment could not be saved.");
    }
    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "intake.stage.assignees",
        "intake_stage",
        Some(id),
    )
    .await;
    Redirect::to(&format!("/admin/intake/stages/{id}")).into_response()
}

pub async fn delete_stage(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::IntakeConfigure) {
        return response;
    }
    if !verify(&jar, &form) {
        return csrf_failure();
    }
    let flows = match load_flows(&state.db, session.organisation_id, true).await {
        Ok(flows) => flows,
        Err(_) => return failure("The stage could not be loaded."),
    };
    let Some((flow, stage)) = find_stage(&flows, id) else {
        return not_found();
    };
    if stage.is_terminal || flow.stages.first().is_some_and(|first| first.id == id) {
        return failure("The first stage or a terminal stage cannot be deleted.");
    }
    let case_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM intake_cases WHERE organisation_id = $1 AND stage_id = $2",
    )
    .bind(session.organisation_id)
    .bind(id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);
    if case_count > 0 {
        return failure("This stage has cases and cannot be deleted.");
    }
    let _ = sqlx::query("DELETE FROM intake_stages WHERE organisation_id = $1 AND id = $2")
        .bind(session.organisation_id)
        .bind(id)
        .execute(&state.db)
        .await;
    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "intake.stage.delete",
        "intake_stage",
        Some(id),
    )
    .await;
    Redirect::to(&format!("/admin/intake/flows/{}", flow.id)).into_response()
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

fn render(session: &AdminSession, body: String) -> String {
    html::layout("Intake", ACTIVE, Some(&session.who()), &body)
}

fn page(session: &AdminSession, body: String) -> Response {
    Html(render(session, body)).into_response()
}

fn failure(message: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Html(html::error_page(
            "Intake configuration",
            &format!("{message} Return to the intake configuration and try again."),
        )),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(html::error_page(
            "Intake record not found",
            "It may have been deleted, or it belongs to another organisation.",
        )),
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

fn subject_kind_options() -> String {
    intake::SUBJECT_KINDS
        .iter()
        .map(|kind| {
            format!(
                "<option value=\"{}\">{}</option>",
                html::escape(kind),
                html::escape(kind)
            )
        })
        .collect()
}

fn stage_movement_options(selected: Option<&str>) -> String {
    [("any", "Any stage"), ("sequential", "Sequential")]
        .into_iter()
        .map(|(value, label)| {
            format!(
                "<option value=\"{value}\" {}>{label}</option>",
                if selected == Some(value) {
                    "selected"
                } else {
                    ""
                }
            )
        })
        .collect()
}

fn stage_kind_options(selected: Option<&str>) -> String {
    STAGE_KINDS
        .iter()
        .map(|kind| {
            format!(
                "<option value=\"{}\" {}>{}</option>",
                html::escape(kind),
                if selected == Some(*kind) {
                    "selected"
                } else {
                    ""
                },
                html::escape(kind)
            )
        })
        .collect()
}

fn lifecycle_options(selected: Option<&str>) -> String {
    ["enquiry", "onboarding", "active", "intake_cancelled"]
        .iter()
        .map(|status| {
            format!(
                "<option value=\"{}\" {}>{}</option>",
                html::escape(status),
                if selected == Some(*status) {
                    "selected"
                } else {
                    ""
                },
                html::escape(status)
            )
        })
        .collect()
}

fn strategy_options(selected: &str) -> String {
    [
        ("fixed", "Fixed user"),
        ("first_available", "Pool — fewest open cases"),
        ("round_robin", "Pool — round robin"),
    ]
    .iter()
    .map(|(value, label)| {
        format!(
            "<option value=\"{}\" {}>{}</option>",
            value,
            if selected == *value { "selected" } else { "" },
            html::escape(label)
        )
    })
    .collect()
}

fn flow_options(flows: &[crate::api::intake::FlowDto], selected: Option<Uuid>) -> String {
    flows
        .iter()
        .map(|flow| {
            format!(
                "<option value=\"{}\" {}>{} ({})</option>",
                flow.id,
                if selected == Some(flow.id) {
                    "selected"
                } else {
                    ""
                },
                html::escape(&flow.name),
                html::escape(&flow.key)
            )
        })
        .collect()
}

fn user_options(users: &[UserRow], selected: &str) -> String {
    users
        .iter()
        .map(|user| {
            let selected = selected == user.id.to_string().as_str();
            format!(
                "<option value=\"{}\" {}>{} · {}</option>",
                user.id,
                if selected { "selected" } else { "" },
                html::escape(&user.full_name),
                html::escape(&user.email)
            )
        })
        .collect()
}

fn pool_options(pools: &[PoolRow], selected: &str) -> String {
    pools
        .iter()
        .map(|pool| {
            let selected = selected == pool.id.to_string().as_str();
            format!(
                "<option value=\"{}\" {}>{}</option>",
                pool.id,
                if selected { "selected" } else { "" },
                html::escape(&pool.name)
            )
        })
        .collect()
}

fn checklist_text(stage: &crate::api::intake::StageDto) -> String {
    stage
        .checklist
        .iter()
        .map(|item| {
            let prefix = if item.is_required { "*" } else { "-" };
            match item.description.as_deref() {
                Some(description) if !description.is_empty() => {
                    format!("{prefix} {} | {description}", item.label)
                }
                _ => format!("{prefix} {}", item.label),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Data helpers
// ---------------------------------------------------------------------------

async fn field_options(
    state: &AppState,
    organisation_id: Uuid,
) -> Result<Vec<FieldOption>, sqlx::Error> {
    let mut options: Vec<FieldOption> = builtin_fields()
        .iter()
        .map(|field| FieldOption {
            key: field.key.to_owned(),
            label: field.label.to_owned(),
            section: field.section.to_owned(),
        })
        .collect();
    let custom = sqlx::query_as::<_, CustomFieldRow>(
        "SELECT key, label
         FROM custom_field_definitions
         WHERE organisation_id = $1 AND entity_type = 'contact' AND is_active
         ORDER BY sort_order, label, key",
    )
    .bind(organisation_id)
    .fetch_all(&state.db)
    .await?;
    for field in custom {
        options.push(FieldOption {
            key: format!("custom:{}", field.key),
            label: field.label,
            section: "Custom fields".to_owned(),
        });
    }
    options.sort_by(|left, right| {
        left.section
            .cmp(&right.section)
            .then(left.label.cmp(&right.label))
    });
    Ok(options)
}

async fn custom_keys(state: &AppState, organisation_id: Uuid) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT key FROM custom_field_definitions
         WHERE organisation_id = $1 AND entity_type = 'contact' AND is_active",
    )
    .bind(organisation_id)
    .fetch_all(&state.db)
    .await
}

async fn users(state: &AppState, organisation_id: Uuid) -> Result<Vec<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(
        "SELECT u.id, u.full_name, u.email
         FROM organisation_users m
         JOIN users u ON u.id = m.user_id
         WHERE m.organisation_id = $1 AND u.is_active
         ORDER BY u.full_name, u.email",
    )
    .bind(organisation_id)
    .fetch_all(&state.db)
    .await
}

async fn pools(state: &AppState, organisation_id: Uuid) -> Result<Vec<PoolRow>, sqlx::Error> {
    sqlx::query_as::<_, PoolRow>(
        "SELECT id, name FROM permission_sets
         WHERE organisation_id = $1 ORDER BY sort_order, name",
    )
    .bind(organisation_id)
    .fetch_all(&state.db)
    .await
}

fn find_stage(
    flows: &[crate::api::intake::FlowDto],
    id: Uuid,
) -> Option<(&crate::api::intake::FlowDto, &crate::api::intake::StageDto)> {
    flows.iter().find_map(|flow| {
        flow.stages
            .iter()
            .find(|stage| stage.id == id)
            .map(|stage| (flow, stage))
    })
}

// ---------------------------------------------------------------------------
// Form helpers
// ---------------------------------------------------------------------------

fn verify(jar: &CookieJar, form: &HashMap<String, String>) -> bool {
    csrf::verify(jar, form.get("csrf").map(String::as_str).unwrap_or(""))
}

fn text(form: &HashMap<String, String>, key: &str) -> String {
    form.get(key)
        .map(|value| value.trim().to_owned())
        .unwrap_or_default()
}

fn optional_text(form: &HashMap<String, String>, key: &str) -> Option<String> {
    form.get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn parse_uuid(value: Option<&String>) -> Option<Uuid> {
    value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .and_then(|value| Uuid::parse_str(value).ok())
}

fn parse_positive_i32(value: Option<&String>) -> Result<Option<i32>, String> {
    let Some(value) = value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let hours = value
        .parse::<i32>()
        .map_err(|_| "SLA hours must be a whole number.".to_owned())?;
    if hours <= 0 {
        return Err("SLA hours must be positive.".to_owned());
    }
    Ok(Some(hours))
}

struct ChecklistInput {
    label: String,
    description: Option<String>,
    is_required: bool,
}

fn parse_checklist(value: &str) -> Vec<ChecklistInput> {
    value
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (is_required, line) = if let Some(rest) = line.strip_prefix('*') {
                (true, rest.trim())
            } else if let Some(rest) = line.strip_prefix('-') {
                (false, rest.trim())
            } else {
                (true, line)
            };
            if line.is_empty() {
                return None;
            }
            let (label, description) = match line.split_once('|') {
                Some((label, description)) => (
                    label.trim().to_owned(),
                    Some(description.trim().to_owned()).filter(|value| !value.is_empty()),
                ),
                None => (line.to_owned(), None),
            };
            Some(ChecklistInput {
                label,
                description,
                is_required,
            })
        })
        .take(100)
        .collect()
}

fn friendly_db_error(error: sqlx::Error) -> String {
    if let sqlx::Error::Database(database) = &error {
        if database.code().as_deref() == Some("23505") {
            return "That key is already in use.".to_owned();
        }
    }
    tracing::warn!(error = %error, "intake admin database error");
    "The configuration could not be saved.".to_owned()
}
