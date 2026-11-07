//! Relationship network page: a server-rendered map of who is connected to whom.
//!
//! Every edge carries a role for each endpoint, so the SVG labels both ends
//! (doctor/patient, guardian/person supported) rather than inventing an arrow
//! direction.

use std::collections::HashMap;
use std::f64::consts::TAU;

use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    admin::{auth::AdminSession, html},
    permissions::Capability,
    relationships::{
        ego_graph, load_edges, whole_graph, GraphNode, NodeKey, PartyKind, RelationshipEdge,
        RelationshipGraph,
    },
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct NetworkParams {
    pub focus_kind: Option<String>,
    pub focus_id: Option<Uuid>,
    pub depth: Option<usize>,
    pub q: Option<String>,
}

#[derive(Debug, FromRow)]
struct SearchParty {
    kind: String,
    id: Uuid,
    label: String,
}

#[derive(Debug, FromRow)]
struct BusyContact {
    id: Uuid,
    display_name: String,
    connections: i64,
}

pub async fn page(
    State(state): State<AppState>,
    session: AdminSession,
    Query(params): Query<NetworkParams>,
) -> Response {
    if let Some(response) = session.guard(Capability::RelationshipsRead) {
        return response;
    }

    let edges = match load_edges(&state.db, session.organisation_id).await {
        Ok(edges) => edges,
        Err(err) => {
            tracing::error!(error = %err, "relationship graph query failed");
            return failure("The relationship network could not be loaded.");
        }
    };

    let focus = match (params.focus_kind.as_deref(), params.focus_id) {
        (Some(kind), Some(id)) => PartyKind::parse(kind).map(|kind| NodeKey { kind, id }),
        _ => None,
    };
    let depth = params.depth.unwrap_or(1).clamp(1, 3);

    let focus_label = match focus {
        Some(focus) => match load_party_label(&state, session.organisation_id, focus).await {
            Ok(label) => label,
            Err(err) => {
                tracing::error!(error = %err, "focused party lookup failed");
                return failure("The selected party could not be loaded.");
            }
        },
        None => None,
    };

    let graph = focus.map_or_else(
        || whole_graph(&edges),
        |focus| ego_graph(&edges, focus, depth),
    );

    let query = params.q.unwrap_or_default().trim().to_owned();
    let matches = if query.is_empty() {
        Vec::new()
    } else {
        match search_parties(&state, session.organisation_id, &query).await {
            Ok(matches) => matches,
            Err(err) => {
                tracing::error!(error = %err, "relationship search failed");
                return failure("The network search could not be completed.");
            }
        }
    };

    let busy = match busy_contacts(&state, session.organisation_id).await {
        Ok(busy) => busy,
        Err(err) => {
            tracing::error!(error = %err, "busy contact query failed");
            return failure("The relationship network could not be loaded.");
        }
    };

    let counts = match counts(&state, session.organisation_id).await {
        Ok(counts) => counts,
        Err(err) => {
            tracing::error!(error = %err, "network counts query failed");
            return failure("The relationship network could not be loaded.");
        }
    };

    Html(render(RenderInput {
        session: &session,
        edges: &edges,
        graph: &graph,
        focus,
        focus_label: focus_label.as_deref(),
        depth,
        query: &query,
        matches: &matches,
        busy: &busy,
        counts,
    }))
    .into_response()
}

struct RenderInput<'a> {
    session: &'a AdminSession,
    edges: &'a [RelationshipEdge],
    graph: &'a RelationshipGraph,
    focus: Option<NodeKey>,
    focus_label: Option<&'a str>,
    depth: usize,
    query: &'a str,
    matches: &'a [SearchParty],
    busy: &'a [BusyContact],
    counts: (i64, i64),
}

fn render(input: RenderInput<'_>) -> String {
    let RenderInput {
        session,
        edges,
        graph,
        focus,
        focus_label,
        depth,
        query,
        matches,
        busy,
        counts,
    } = input;
    let (contacts, organisations) = counts;

    let mut body = String::new();
    body.push_str(&html::page_head(
        "Relationship network",
        "Every relationship stores a role for both sides. Search for a person or organisation to inspect their connections, or explore the whole network.",
    ));

    body.push_str("<div class=\"grid\">");
    body.push_str(&metric("Contacts", &contacts.to_string()));
    body.push_str(&metric("Organisations", &organisations.to_string()));
    body.push_str(&metric("Two-sided links", &edges.len().to_string()));
    body.push_str(&metric(
        "Showing",
        &format!("{} nodes · {} edges", graph.nodes.len(), graph.edges.len()),
    ));
    body.push_str("</div>");

    body.push_str(
        "<form class=\"form wide\" method=\"get\" action=\"/admin/relationships\" style=\"margin-top:18px\">\
         <label for=\"q\">Search people or organisations</label>\
         <input id=\"q\" name=\"q\" type=\"search\" value=\"",
    );
    body.push_str(&html::escape(query));
    body.push_str(
        "\" placeholder=\"Name, email or phone\">\
         <button type=\"submit\">Search network</button></form>",
    );

    if !query.is_empty() {
        body.push_str("<h2>Search results</h2>");
        if matches.is_empty() {
            body.push_str("<p class=\"sub\">No matching parties.</p>");
        } else {
            body.push_str(
                "<table><thead><tr><th>Name</th><th>Kind</th><th></th></tr></thead><tbody>",
            );
            for party in matches {
                body.push_str(&format!(
                    "<tr><td>{}</td><td><span class=\"badge {}\">{}</span></td>\
                     <td><a href=\"/admin/relationships?focus_kind={}&focus_id={}&depth={depth}\">Focus graph</a></td></tr>",
                    html::escape(&party.label),
                    if party.kind == "contact" { "pk" } else { "fk" },
                    html::escape(&party.kind),
                    html::escape(&party.kind),
                    party.id,
                ));
            }
            body.push_str("</tbody></table>");
        }
    }

    body.push_str("<h2>Network</h2>");

    if let Some(focus) = focus {
        let label = focus_label.unwrap_or("Unknown party");
        body.push_str(&format!(
            "<p class=\"sub\">Focused on <strong>{}</strong> ({}), showing {} hop{}. \
             <a href=\"/admin/relationships\">Show the whole network</a>.</p>",
            html::escape(label),
            focus.kind.as_str(),
            depth,
            if depth == 1 { "" } else { "s" },
        ));

        body.push_str("<p class=\"sub\">Depth: ");
        for candidate in 1..=3 {
            if candidate == depth {
                body.push_str(&format!("<strong>{candidate}</strong>"));
            } else {
                body.push_str(&format!(
                    "<a href=\"/admin/relationships?focus_kind={}&focus_id={}&depth={candidate}\">{candidate}</a>",
                    focus.kind.as_str(),
                    focus.id
                ));
            }
            if candidate < 3 {
                body.push_str(" · ");
            }
        }
        body.push_str("</p>");

        if let Some(focus_label) = focus_label {
            body.push_str(&connections_table(edges, focus, focus_label));
        }
    } else {
        body.push_str(
            "<p class=\"sub\">Whole tenant network. Organisations sit on the inner ring; contacts are spread across outer rings. \
             Colours: <span class=\"badge pk\">contact</span> <span class=\"badge fk\">organisation</span>. \
             Hover a dot for its name. Search or focus a party for a readable map.</p>",
        );
    }

    if graph.nodes.is_empty() {
        body.push_str(
            "<p class=\"sub\">No relationships have been recorded for this organisation yet.</p>",
        );
    } else {
        body.push_str("<div class=\"network\">");
        body.push_str(&render_svg(graph, focus));
        body.push_str("</div>");
    }

    if focus.is_none() && !busy.is_empty() {
        body.push_str("<h2>Busiest connections</h2>");
        body.push_str(
            "<table><thead><tr><th>Contact</th><th>Connections</th><th></th></tr></thead><tbody>",
        );
        for contact in busy {
            body.push_str(&format!(
                "<tr><td>{}</td><td>{}</td>\
                 <td><a href=\"/admin/relationships?focus_kind=contact&focus_id={}&depth=2\">Focus graph</a></td></tr>",
                html::escape(&contact.display_name),
                contact.connections,
                contact.id,
            ));
        }
        body.push_str("</tbody></table>");
    }

    html::layout(
        "Relationship network",
        "/admin/relationships",
        Some(&session.who()),
        &body,
    )
}

fn connections_table(edges: &[RelationshipEdge], focus: NodeKey, focus_label: &str) -> String {
    let mut rows = edges
        .iter()
        .filter_map(|edge| {
            if edge.from_key() == focus {
                Some((
                    &edge.to_name,
                    edge.to_kind,
                    edge.to_role.as_str(),
                    edge.from_role.as_str(),
                ))
            } else if edge.to_key() == focus {
                Some((
                    &edge.from_name,
                    edge.from_kind,
                    edge.from_role.as_str(),
                    edge.to_role.as_str(),
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    rows.sort_by_key(|row| row.0.to_lowercase());

    let mut out = String::new();
    out.push_str(&format!(
        "<h2>Connections for {}</h2>",
        html::escape(focus_label)
    ));
    if rows.is_empty() {
        out.push_str("<p class=\"sub\">This party has no recorded connections.</p>");
        return out;
    }

    out.push_str(
        "<table><thead><tr><th>Counterpart</th><th>Kind</th><th>Their role</th><th>This party's role</th></tr></thead><tbody>",
    );
    for (name, kind, their_role, our_role) in rows {
        out.push_str(&format!(
            "<tr><td>{}</td><td><span class=\"badge {}\">{}</span></td><td>{}</td><td>{}</td></tr>",
            html::escape(name),
            if kind == PartyKind::Contact {
                "pk"
            } else {
                "fk"
            },
            kind.as_str(),
            html::escape(their_role),
            html::escape(our_role),
        ));
    }
    out.push_str("</tbody></table>");
    out
}

fn render_svg(graph: &RelationshipGraph, focus: Option<NodeKey>) -> String {
    let (positions, width) = if focus.is_some() {
        focus_layout(graph)
    } else {
        overview_layout(graph)
    };
    let height = width;
    let centre = width / 2.0;

    let mut svg = String::with_capacity(32_000);
    svg.push_str(&format!(
        "<svg viewBox=\"0 0 {width:.0} {height:.0}\" width=\"{width:.0}\" height=\"{height:.0}\" \
         role=\"img\" aria-label=\"Relationship network graph\">"
    ));

    let labelled_edges = focus.is_some() && graph.edges.len() <= 70;

    for edge in &graph.edges {
        let (Some(from), Some(to)) = (
            positions.get(&edge.from_key()),
            positions.get(&edge.to_key()),
        ) else {
            continue;
        };
        svg.push_str(&format!(
            "<line class=\"network-edge{}\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\">\
             <title>{} — {} / {} — {}</title></line>",
            if focus.is_some() { " near" } else { "" },
            from.0,
            from.1,
            to.0,
            to.1,
            html::escape(&edge.from_name),
            html::escape(&edge.from_role),
            html::escape(&edge.to_name),
            html::escape(&edge.to_role),
        ));
    }

    if labelled_edges {
        for edge in &graph.edges {
            let (Some(from), Some(to)) = (
                positions.get(&edge.from_key()),
                positions.get(&edge.to_key()),
            ) else {
                continue;
            };
            let (from_t, to_t) = match focus {
                Some(focus) if edge.from_key() == focus => (0.42, 0.86),
                Some(_) => (0.14, 0.58),
                None => (0.18, 0.82),
            };
            push_role_label(&mut svg, from, to, from_t, &edge.from_role);
            push_role_label(&mut svg, from, to, to_t, &edge.to_role);
        }
    }

    for node in &graph.nodes {
        let Some((x, y)) = positions.get(&NodeKey {
            kind: node.kind,
            id: node.id,
        }) else {
            continue;
        };
        let is_focus = focus.is_some_and(|focus| focus.kind == node.kind && focus.id == node.id);
        let level = node.depth.unwrap_or(0);
        let title = format!(
            "{} — {} — {} connection{}",
            node.label,
            node.kind.as_str(),
            node.degree,
            if node.degree == 1 { "" } else { "s" }
        );

        if focus.is_some() {
            svg.push_str(&format!(
                "<g><circle class=\"network-node {} {}\" cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"{}\">\
                 <title>{}</title></circle>",
                if node.kind == PartyKind::Contact {
                    "contact"
                } else {
                    "organisation"
                },
                if is_focus { "focus" } else { "" },
                if is_focus {
                    14.0
                } else if level == 1 {
                    9.0
                } else if level == 2 {
                    5.0
                } else {
                    3.5
                },
                html::escape(&title),
            ));
            if level <= 1 {
                let (label_x, label_y, anchor) = focus_label_position(*x, *y, centre, is_focus);
                svg.push_str(&format!(
                    "<text class=\"network-label\" x=\"{label_x:.1}\" y=\"{label_y:.1}\" text-anchor=\"{anchor}\">{}</text>",
                    html::escape(&truncate(&node.label, 26)),
                ));
            }
            svg.push_str("</g>");
        } else {
            svg.push_str(&format!(
                "<circle class=\"network-node {}\" cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"2.4\">\
                 <title>{}</title></circle>",
                if node.kind == PartyKind::Contact {
                    "contact"
                } else {
                    "organisation"
                },
                html::escape(&title),
            ));
            if node.kind == PartyKind::Organisation {
                svg.push_str(&format!(
                    "<text class=\"network-label small\" x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text>",
                    y - 8.0,
                    html::escape(&truncate(&node.label, 22)),
                ));
            }
        }
    }

    svg.push_str(&format!(
        "<circle cx=\"{centre:.0}\" cy=\"{centre:.0}\" r=\"2\" fill=\"transparent\"/></svg>"
    ));
    svg
}

fn focus_layout(graph: &RelationshipGraph) -> (HashMap<NodeKey, (f64, f64)>, f64) {
    let mut by_depth: HashMap<usize, Vec<&GraphNode>> = HashMap::new();
    for node in &graph.nodes {
        by_depth
            .entry(node.depth.unwrap_or(0))
            .or_default()
            .push(node);
    }

    let max_depth = by_depth.keys().copied().max().unwrap_or(1).max(1);
    let mut radii: HashMap<usize, f64> = HashMap::new();
    let mut previous = 0.0;
    let mut max_radius = 0.0f64;
    for depth in 1..=max_depth {
        let count = by_depth.get(&depth).map_or(0, Vec::len);
        let base = 210.0 * depth as f64;
        let needed = (count as f64 * 52.0 / TAU).min(1200.0);
        let radius = base.max(needed).max(previous + 150.0);
        radii.insert(depth, radius);
        previous = radius;
        max_radius = max_radius.max(radius);
    }

    let size = 2.0 * (max_radius + 220.0);
    let centre = size / 2.0;
    let mut positions = HashMap::new();

    for (depth, mut nodes) in by_depth {
        nodes.sort_by_key(|node| node.label.to_lowercase());
        let radius = radii.get(&depth).copied().unwrap_or(0.0);
        let count = nodes.len().max(1);
        let offset = -TAU / 4.0 + depth as f64 * 0.35;
        for (index, node) in nodes.iter().enumerate() {
            let angle = offset + TAU * index as f64 / count as f64;
            positions.insert(
                NodeKey {
                    kind: node.kind,
                    id: node.id,
                },
                (centre + radius * angle.cos(), centre + radius * angle.sin()),
            );
        }
    }

    (positions, size)
}

fn overview_layout(graph: &RelationshipGraph) -> (HashMap<NodeKey, (f64, f64)>, f64) {
    let size = 1600.0;
    let centre = size / 2.0;
    let mut organisations = graph
        .nodes
        .iter()
        .filter(|node| node.kind == PartyKind::Organisation)
        .collect::<Vec<_>>();
    let mut contacts = graph
        .nodes
        .iter()
        .filter(|node| node.kind == PartyKind::Contact)
        .collect::<Vec<_>>();

    organisations.sort_by(|a, b| {
        b.degree
            .cmp(&a.degree)
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });
    contacts.sort_by(|a, b| {
        b.degree
            .cmp(&a.degree)
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });

    let mut positions = HashMap::new();
    place_ring(&mut positions, &organisations, centre, 300.0, -TAU / 4.0);

    let per_ring = contacts.len().div_ceil(3).max(1);
    for (ring, chunk) in contacts.chunks(per_ring).enumerate() {
        let radius = 600.0 + ring as f64 * 70.0;
        place_ring(
            &mut positions,
            chunk,
            centre,
            radius,
            -TAU / 4.0 + 0.02 * ring as f64,
        );
    }

    (positions, size)
}

fn place_ring(
    positions: &mut HashMap<NodeKey, (f64, f64)>,
    nodes: &[&GraphNode],
    centre: f64,
    radius: f64,
    offset: f64,
) {
    let count = nodes.len().max(1);
    for (index, node) in nodes.iter().enumerate() {
        let angle = offset + TAU * index as f64 / count as f64;
        positions.insert(
            NodeKey {
                kind: node.kind,
                id: node.id,
            },
            (centre + radius * angle.cos(), centre + radius * angle.sin()),
        );
    }
}

fn focus_label_position(x: f64, y: f64, centre: f64, is_focus: bool) -> (f64, f64, &'static str) {
    if is_focus {
        return (x, y + 34.0, "middle");
    }

    let dx = x - centre;
    let dy = y - centre;
    let distance = (dx * dx + dy * dy).sqrt();
    if distance < 1.0 {
        return (x, y + 22.0, "middle");
    }

    let ux = dx / distance;
    let uy = dy / distance;
    let anchor = if ux > 0.3 {
        "start"
    } else if ux < -0.3 {
        "end"
    } else {
        "middle"
    };

    (x + ux * 18.0, y + uy * 18.0 + 4.0, anchor)
}

fn push_role_label(svg: &mut String, from: &(f64, f64), to: &(f64, f64), t: f64, role: &str) {
    let x = from.0 + (to.0 - from.0) * t;
    let y = from.1 + (to.1 - from.1) * t;
    svg.push_str(&format!(
        "<text class=\"network-role\" x=\"{x:.1}\" y=\"{y:.1}\" text-anchor=\"middle\">{}</text>",
        html::escape(role)
    ));
}

async fn load_party_label(
    state: &AppState,
    organisation_id: Uuid,
    focus: NodeKey,
) -> Result<Option<String>, sqlx::Error> {
    match focus.kind {
        PartyKind::Contact => {
            sqlx::query_scalar(
                "SELECT display_name FROM contacts WHERE organisation_id=$1 AND id=$2",
            )
            .bind(organisation_id)
            .bind(focus.id)
            .fetch_optional(&state.db)
            .await
        }
        PartyKind::Organisation => {
            sqlx::query_scalar("SELECT name FROM organisations WHERE tenant_id=$1 AND id=$2")
                .bind(organisation_id)
                .bind(focus.id)
                .fetch_optional(&state.db)
                .await
        }
    }
}

async fn search_parties(
    state: &AppState,
    organisation_id: Uuid,
    query: &str,
) -> Result<Vec<SearchParty>, sqlx::Error> {
    let pattern = format!("%{query}%");
    let contacts = sqlx::query_as::<_, SearchParty>(
        "SELECT 'contact' AS kind, id, display_name AS label
         FROM contacts
         WHERE organisation_id=$1
           AND (display_name ILIKE $2 OR email ILIKE $2 OR phone ILIKE $2)
         ORDER BY display_name
         LIMIT 10",
    )
    .bind(organisation_id)
    .bind(&pattern)
    .fetch_all(&state.db)
    .await?;

    let organisations = sqlx::query_as::<_, SearchParty>(
        "SELECT 'organisation' AS kind, id, name AS label
         FROM organisations
         WHERE tenant_id=$1 AND name ILIKE $2
         ORDER BY name
         LIMIT 10",
    )
    .bind(organisation_id)
    .bind(&pattern)
    .fetch_all(&state.db)
    .await?;

    let mut parties = contacts;
    parties.extend(organisations);
    Ok(parties)
}

async fn busy_contacts(
    state: &AppState,
    organisation_id: Uuid,
) -> Result<Vec<BusyContact>, sqlx::Error> {
    sqlx::query_as::<_, BusyContact>(
        "SELECT c.id, c.display_name, count(r.id) AS connections
         FROM contacts c
         JOIN relationships r
           ON r.organisation_id = c.organisation_id
          AND (r.from_contact_id = c.id OR r.to_contact_id = c.id)
         WHERE c.organisation_id = $1
         GROUP BY c.id, c.display_name
         ORDER BY connections DESC, c.display_name
         LIMIT 8",
    )
    .bind(organisation_id)
    .fetch_all(&state.db)
    .await
}

async fn counts(state: &AppState, organisation_id: Uuid) -> Result<(i64, i64), sqlx::Error> {
    let contacts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM contacts WHERE organisation_id=$1")
            .bind(organisation_id)
            .fetch_one(&state.db)
            .await?;
    let organisations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM organisations WHERE tenant_id=$1")
            .bind(organisation_id)
            .fetch_one(&state.db)
            .await?;
    Ok((contacts, organisations))
}

fn metric(label: &str, value: &str) -> String {
    format!(
        "<div class=\"card\"><p class=\"sub\">{}</p><p class=\"big\">{}</p></div>",
        html::escape(label),
        html::escape(value)
    )
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

fn failure(message: &str) -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page("Relationship network", message)),
    )
        .into_response()
}
