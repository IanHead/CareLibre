//! Two-sided relationship queries and graph construction.
//!
//! A relationship row stores one role for each endpoint. Reading left to right
//! uses `from_role`; reading right to left uses `to_role`. The graph therefore
//! carries both labels on every edge instead of inventing an inverse.

use std::collections::{BTreeMap, HashMap, VecDeque};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use utoipa::ToSchema;
use uuid::Uuid;

/// The kind of party at one end of a relationship.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PartyKind {
    Contact,
    Organisation,
}

impl PartyKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::Organisation => "organisation",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "contact" => Some(Self::Contact),
            "organisation" | "organization" => Some(Self::Organisation),
            _ => None,
        }
    }
}

/// One relationship with both endpoint roles and display names.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RelationshipEdge {
    pub id: Uuid,
    pub from_kind: PartyKind,
    pub from_id: Uuid,
    pub from_name: String,
    pub from_role: String,
    pub to_kind: PartyKind,
    pub to_id: Uuid,
    pub to_name: String,
    pub to_role: String,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub notes: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// A party in the graph. `degree` counts incident relationships.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GraphNode {
    pub kind: PartyKind,
    pub id: Uuid,
    pub label: String,
    pub degree: usize,
    /// Distance from the focused node, or `None` for a whole-network view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
}

/// Nodes and edges ready for rendering or for a client-side graph library.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RelationshipGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<RelationshipEdge>,
}

/// Composite identity for a party across the two endpoint tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeKey {
    pub kind: PartyKind,
    pub id: Uuid,
}

impl RelationshipEdge {
    #[must_use]
    pub fn from_key(&self) -> NodeKey {
        NodeKey {
            kind: self.from_kind,
            id: self.from_id,
        }
    }

    #[must_use]
    pub fn to_key(&self) -> NodeKey {
        NodeKey {
            kind: self.to_kind,
            id: self.to_id,
        }
    }

    #[must_use]
    pub fn other(&self, key: NodeKey) -> Option<NodeKey> {
        if self.from_key() == key {
            Some(self.to_key())
        } else if self.to_key() == key {
            Some(self.from_key())
        } else {
            None
        }
    }
}

#[derive(Debug, FromRow)]
struct EdgeRow {
    id: Uuid,
    from_kind: String,
    from_id: Uuid,
    from_name: String,
    from_role: String,
    to_kind: String,
    to_id: Uuid,
    to_name: String,
    to_role: String,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
    notes: Option<String>,
    updated_at: DateTime<Utc>,
}

const EDGE_SELECT: &str = r"
SELECT r.id,
       CASE WHEN r.from_contact_id IS NOT NULL THEN 'contact' ELSE 'organisation' END AS from_kind,
       COALESCE(r.from_contact_id, r.from_organisation_id) AS from_id,
       COALESCE(fc.display_name, fo.name, 'Unknown') AS from_name,
       r.from_role,
       CASE WHEN r.to_contact_id IS NOT NULL THEN 'contact' ELSE 'organisation' END AS to_kind,
       COALESCE(r.to_contact_id, r.to_organisation_id) AS to_id,
       COALESCE(tc.display_name, to_org.name, 'Unknown') AS to_name,
       r.to_role,
       r.start_date,
       r.end_date,
       r.notes,
       r.updated_at
FROM relationships r
LEFT JOIN contacts fc
       ON fc.organisation_id = r.organisation_id AND fc.id = r.from_contact_id
LEFT JOIN organisations fo
       ON fo.tenant_id = r.organisation_id AND fo.id = r.from_organisation_id
LEFT JOIN contacts tc
       ON tc.organisation_id = r.organisation_id AND tc.id = r.to_contact_id
LEFT JOIN organisations to_org
       ON to_org.tenant_id = r.organisation_id AND to_org.id = r.to_organisation_id
WHERE r.organisation_id = $1
";

/// Load every relationship for a tenant, with names and both roles.
pub async fn load_edges(
    db: &PgPool,
    organisation_id: Uuid,
) -> Result<Vec<RelationshipEdge>, sqlx::Error> {
    let rows = sqlx::query_as::<_, EdgeRow>(&format!(
        "{EDGE_SELECT} ORDER BY lower(COALESCE(fc.display_name, fo.name)), r.from_role, \
         lower(COALESCE(tc.display_name, to_org.name)), r.to_role, r.id"
    ))
    .bind(organisation_id)
    .fetch_all(db)
    .await?;

    Ok(rows.into_iter().map(edge_from_row).collect())
}

/// Load one relationship by id, scoped to the tenant.
pub async fn load_edge(
    db: &PgPool,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<Option<RelationshipEdge>, sqlx::Error> {
    let row = sqlx::query_as::<_, EdgeRow>(&format!("{EDGE_SELECT} AND r.id = $2"))
        .bind(organisation_id)
        .bind(id)
        .fetch_optional(db)
        .await?;

    Ok(row.map(edge_from_row))
}

fn edge_from_row(row: EdgeRow) -> RelationshipEdge {
    RelationshipEdge {
        id: row.id,
        from_kind: PartyKind::parse(&row.from_kind).unwrap_or(PartyKind::Contact),
        from_id: row.from_id,
        from_name: row.from_name,
        from_role: row.from_role,
        to_kind: PartyKind::parse(&row.to_kind).unwrap_or(PartyKind::Contact),
        to_id: row.to_id,
        to_name: row.to_name,
        to_role: row.to_role,
        start_date: row.start_date,
        end_date: row.end_date,
        notes: row.notes,
        updated_at: row.updated_at,
    }
}

/// Build the whole-network graph for a tenant.
#[must_use]
pub fn whole_graph(edges: &[RelationshipEdge]) -> RelationshipGraph {
    graph_from_edges(edges, None, None)
}

/// Build the network around one party, up to `depth` hops away.
#[must_use]
pub fn ego_graph(edges: &[RelationshipEdge], focus: NodeKey, depth: usize) -> RelationshipGraph {
    graph_from_edges(edges, Some(focus), Some(depth.max(1)))
}

/// Filter edges by either endpoint. `None` means "no filter on this side".
#[must_use]
pub fn filter_edges(
    edges: &[RelationshipEdge],
    contact_id: Option<Uuid>,
    organisation_id: Option<Uuid>,
) -> Vec<RelationshipEdge> {
    edges
        .iter()
        .filter(|edge| {
            let matches_contact = contact_id.is_none_or(|id| {
                (edge.from_kind == PartyKind::Contact && edge.from_id == id)
                    || (edge.to_kind == PartyKind::Contact && edge.to_id == id)
            });
            let matches_organisation = organisation_id.is_none_or(|id| {
                (edge.from_kind == PartyKind::Organisation && edge.from_id == id)
                    || (edge.to_kind == PartyKind::Organisation && edge.to_id == id)
            });
            matches_contact && matches_organisation
        })
        .cloned()
        .collect()
}

fn graph_from_edges(
    edges: &[RelationshipEdge],
    focus: Option<NodeKey>,
    depth: Option<usize>,
) -> RelationshipGraph {
    let mut adjacency: HashMap<NodeKey, Vec<usize>> = HashMap::new();
    for (index, edge) in edges.iter().enumerate() {
        adjacency.entry(edge.from_key()).or_default().push(index);
        adjacency.entry(edge.to_key()).or_default().push(index);
    }

    let (included, depths) = match (focus, depth) {
        (Some(focus), Some(depth)) => {
            let mut depths = HashMap::from([(focus, 0usize)]);
            let mut queue = VecDeque::from([focus]);
            while let Some(node) = queue.pop_front() {
                let current = depths[&node];
                if current >= depth {
                    continue;
                }
                if let Some(indexes) = adjacency.get(&node) {
                    for index in indexes {
                        if let Some(next) = edges[*index].other(node) {
                            if let std::collections::hash_map::Entry::Vacant(entry) =
                                depths.entry(next)
                            {
                                entry.insert(current + 1);
                                queue.push_back(next);
                            }
                        }
                    }
                }
            }
            (depths.keys().copied().collect::<Vec<_>>(), depths)
        }
        _ => (adjacency.keys().copied().collect(), HashMap::new()),
    };

    let included_set: std::collections::HashSet<NodeKey> = included.iter().copied().collect();
    let mut nodes: BTreeMap<NodeKey, GraphNode> = BTreeMap::new();
    let mut degrees: HashMap<NodeKey, usize> = HashMap::new();
    let mut kept_edges = Vec::new();

    for edge in edges {
        let from = edge.from_key();
        let to = edge.to_key();
        if !included_set.contains(&from) || !included_set.contains(&to) {
            continue;
        }

        *degrees.entry(from).or_default() += 1;
        *degrees.entry(to).or_default() += 1;
        kept_edges.push(edge.clone());

        nodes.entry(from).or_insert_with(|| GraphNode {
            kind: from.kind,
            id: from.id,
            label: edge.from_name.clone(),
            degree: 0,
            depth: depths.get(&from).copied(),
        });
        nodes.entry(to).or_insert_with(|| GraphNode {
            kind: to.kind,
            id: to.id,
            label: edge.to_name.clone(),
            degree: 0,
            depth: depths.get(&to).copied(),
        });
    }

    for (key, node) in &mut nodes {
        node.degree = degrees.get(key).copied().unwrap_or(0);
    }

    // A focused party with no relationships should still appear as a lone node.
    if let Some(focus) = focus {
        if let std::collections::btree_map::Entry::Vacant(entry) = nodes.entry(focus) {
            if let Some(edge) = edges
                .iter()
                .find(|edge| edge.from_key() == focus || edge.to_key() == focus)
            {
                let (label, kind) = if edge.from_key() == focus {
                    (edge.from_name.clone(), edge.from_kind)
                } else {
                    (edge.to_name.clone(), edge.to_kind)
                };
                entry.insert(GraphNode {
                    kind,
                    id: focus.id,
                    label,
                    degree: 0,
                    depth: Some(0),
                });
            }
        }
    }

    let mut nodes = nodes.into_values().collect::<Vec<_>>();
    nodes.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| b.degree.cmp(&a.degree))
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });

    RelationshipGraph {
        nodes,
        edges: kept_edges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(id: u128, from: (PartyKind, Uuid), to: (PartyKind, Uuid)) -> RelationshipEdge {
        RelationshipEdge {
            id: Uuid::from_u128(id),
            from_kind: from.0,
            from_id: from.1,
            from_name: format!("from-{id}"),
            from_role: "doctor".to_owned(),
            to_kind: to.0,
            to_id: to.1,
            to_name: format!("to-{id}"),
            to_role: "patient".to_owned(),
            start_date: None,
            end_date: None,
            notes: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn parses_party_kinds() {
        assert_eq!(PartyKind::parse("contact"), Some(PartyKind::Contact));
        assert_eq!(
            PartyKind::parse(" OrganisATION "),
            Some(PartyKind::Organisation)
        );
        assert_eq!(PartyKind::parse("user"), None);
    }

    #[test]
    fn ego_graph_respects_depth() {
        let a = (PartyKind::Contact, Uuid::from_u128(1));
        let b = (PartyKind::Contact, Uuid::from_u128(2));
        let c = (PartyKind::Contact, Uuid::from_u128(3));
        let edges = vec![edge(10, a, b), edge(11, b, c)];
        let focus = NodeKey {
            kind: PartyKind::Contact,
            id: a.1,
        };

        let one = ego_graph(&edges, focus, 1);
        assert_eq!(one.nodes.len(), 2);
        assert_eq!(one.edges.len(), 1);

        let two = ego_graph(&edges, focus, 2);
        assert_eq!(two.nodes.len(), 3);
        assert_eq!(two.edges.len(), 2);
    }

    #[test]
    fn edges_keep_both_roles() {
        let edge = edge(
            10,
            (PartyKind::Contact, Uuid::from_u128(1)),
            (PartyKind::Contact, Uuid::from_u128(2)),
        );
        assert_eq!(edge.from_role, "doctor");
        assert_eq!(edge.to_role, "patient");
    }
}
