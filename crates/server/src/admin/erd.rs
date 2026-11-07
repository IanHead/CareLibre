//! Minimal server-side SVG entity-relationship diagram.
//!
//! Cards are laid out on a fixed grid; relationships are drawn as bezier curves
//! *behind* the cards, so lines appear to connect card edges.

use std::collections::HashMap;

use crate::admin::{html, schema::SchemaModel};

const CARD_W: i32 = 300;
const GAP_X: i32 = 80;
const GAP_Y: i32 = 72;
const MARGIN: i32 = 28;
const HEADER_H: i32 = 34;
const ROW_H: i32 = 19;
const PAD_BOTTOM: i32 = 12;
const MAX_COLS: usize = 3;

pub fn render(model: &SchemaModel) -> String {
    let tables = &model.tables;
    if tables.is_empty() {
        return "<p class=\"sub\">No tables found in the public schema.</p>".to_owned();
    }

    let columns = MAX_COLS.min(tables.len()).max(1);
    let rows = tables.len().div_ceil(columns);

    let heights: Vec<i32> = tables
        .iter()
        .map(|table| HEADER_H + table.columns.len() as i32 * ROW_H + PAD_BOTTOM)
        .collect();

    let mut row_heights = vec![0i32; rows];
    for (position, height) in heights.iter().enumerate() {
        let row = position / columns;
        row_heights[row] = row_heights[row].max(*height);
    }

    let mut positions: HashMap<&str, (i32, i32, i32)> = HashMap::with_capacity(tables.len());
    let mut y = MARGIN;
    for (row, row_height) in row_heights.iter().enumerate() {
        for column in 0..columns {
            let position = row * columns + column;
            if position >= tables.len() {
                break;
            }
            let x = MARGIN + column as i32 * (CARD_W + GAP_X);
            positions.insert(tables[position].name.as_str(), (x, y, heights[position]));
        }
        y += row_height + GAP_Y;
    }

    let width = MARGIN * 2 + columns as i32 * CARD_W + (columns as i32 - 1) * GAP_X;
    let height = y - GAP_Y + MARGIN;

    let mut svg = String::with_capacity(16_000);
    svg.push_str(&format!(
        "<svg viewBox=\"0 0 {width} {height}\" width=\"{width}\" height=\"{height}\" role=\"img\" aria-label=\"Entity relationship diagram\">"
    ));
    svg.push_str(
        "<defs><marker id=\"erd-arrow\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" \
         markerWidth=\"7\" markerHeight=\"7\" orient=\"auto-start-reverse\">\
         <path d=\"M0,0 L8,4 L0,8 z\" fill=\"currentColor\"/></marker></defs>",
    );

    // Relationships first so cards paint over the line ends.
    svg.push_str("<g color=\"#98a1b0\">");
    for table in tables {
        let Some(&(x1, y1, h1)) = positions.get(table.name.as_str()) else {
            continue;
        };
        let mut drawn_targets = std::collections::HashSet::new();
        for foreign_key in &table.foreign_keys {
            // Several constraints can point at the same table (e.g. a
            // composite tenancy FK plus the real reference); draw one edge.
            if !drawn_targets.insert(foreign_key.target_table.as_str()) {
                continue;
            }
            let Some(&(x2, y2, h2)) = positions.get(foreign_key.target_table.as_str()) else {
                continue;
            };

            let start_x = x1 + CARD_W;
            let start_y = y1 + h1 / 2;
            let end_x = x2;
            let end_y = y2 + h2 / 2;
            let bend = ((end_x - start_x).abs() / 2).max(50);

            svg.push_str(&format!(
                "<path class=\"erd-edge\" marker-end=\"url(#erd-arrow)\" \
                 d=\"M {start_x} {start_y} C {} {start_y}, {} {end_y}, {end_x} {end_y}\"/>",
                start_x + bend,
                end_x - bend
            ));
        }
    }
    svg.push_str("</g>");

    for table in tables {
        let Some(&(x, card_y, card_h)) = positions.get(table.name.as_str()) else {
            continue;
        };

        svg.push_str(&format!(
            "<g><rect class=\"erd-card\" x=\"{x}\" y=\"{card_y}\" width=\"{CARD_W}\" height=\"{card_h}\" rx=\"8\"/>"
        ));
        svg.push_str(&format!(
            "<text class=\"erd-title\" x=\"{}\" y=\"{}\">{}</text>",
            x + 14,
            card_y + 22,
            html::escape(&table.name)
        ));
        svg.push_str(&format!(
            "<line class=\"erd-rule\" x1=\"{x}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
            card_y + HEADER_H - 7,
            x + CARD_W,
            card_y + HEADER_H - 7
        ));

        for (index, column) in table.columns.iter().enumerate() {
            let row_y = card_y + HEADER_H + index as i32 * ROW_H + 13;
            let is_pk = table.primary_key.contains(&column.name);
            let is_fk = table
                .foreign_keys
                .iter()
                .any(|foreign_key| foreign_key.column == column.name);

            if is_pk {
                svg.push_str(&format!(
                    "<circle class=\"erd-pk\" cx=\"{}\" cy=\"{}\" r=\"2.6\"/>",
                    x + 12,
                    row_y - 4
                ));
            }
            if is_fk {
                svg.push_str(&format!(
                    "<circle class=\"erd-fk\" cx=\"{}\" cy=\"{}\" r=\"2.6\"/>",
                    x + if is_pk { 22 } else { 12 },
                    row_y - 4
                ));
            }

            let name_x = x + if is_pk || is_fk { 32 } else { 14 };
            svg.push_str(&format!(
                "<text class=\"erd-col\" x=\"{name_x}\" y=\"{row_y}\">{}</text>",
                html::escape(&truncate(&column.name, 22))
            ));
            svg.push_str(&format!(
                "<text class=\"erd-type\" x=\"{}\" y=\"{row_y}\" text-anchor=\"end\">{}</text>",
                x + CARD_W - 14,
                html::escape(&truncate(&column.type_name, 16))
            ));
        }

        svg.push_str("</g>");
    }

    svg.push_str("</svg>");
    svg
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
    use crate::admin::schema::{ColumnModel, TableModel};

    fn table(name: &str, columns: &[&str]) -> TableModel {
        TableModel {
            name: name.to_owned(),
            comment: None,
            columns: columns
                .iter()
                .map(|column| ColumnModel {
                    name: (*column).to_owned(),
                    type_name: "uuid".to_owned(),
                    nullable: false,
                    default: None,
                    identity: false,
                })
                .collect(),
            primary_key: vec!["id".to_owned()],
            foreign_keys: Vec::new(),
            indexes: Vec::new(),
        }
    }

    #[test]
    fn renders_tables_and_svg() {
        let model = SchemaModel {
            tables: vec![table("users", &["id", "email"]), table("sessions", &["id"])],
        };

        let svg = render(&model);
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("users"));
        assert!(svg.contains("sessions"));
        assert!(svg.contains("erd-card"));
    }

    #[test]
    fn empty_model_has_placeholder() {
        let svg = render(&SchemaModel::default());
        assert!(svg.contains("No tables"));
    }

    #[test]
    fn truncates_names() {
        assert_eq!(truncate("short", 8), "short");
        assert_eq!(truncate("0123456789", 5), "0123…");
    }
}
