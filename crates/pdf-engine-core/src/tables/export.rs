//! Serialization and multi-format semantic export for detected tables.
//!
//! Provides RFC 4180 compliant CSV, structured JSON, GitHub Flavored Markdown,
//! and semantic HTML table exporters without external dependencies.

use crate::tables::types::{DetectedTable, TableExportFormat};

/// Exports a detected table to the specified target format string.
pub fn export_table(table: &DetectedTable, format: TableExportFormat) -> String {
    match format {
        TableExportFormat::Csv => export_to_csv(table),
        TableExportFormat::Json => export_to_json(table),
        TableExportFormat::Markdown => export_to_markdown(table),
        TableExportFormat::Html => export_to_html(table),
    }
}

/// Serializes a table into RFC 4180 compliant CSV.
pub fn export_to_csv(table: &DetectedTable) -> String {
    let mut out = String::new();

    // Reconstruct full row list: headers followed by rows
    let mut all_rows: Vec<Vec<String>> = Vec::new();
    if !table.headers.is_empty() {
        all_rows.push(table.headers.clone());
    }
    all_rows.extend(table.rows.clone());

    for (row_idx, row) in all_rows.iter().enumerate() {
        for (col_idx, val) in row.iter().enumerate() {
            if col_idx > 0 {
                out.push(',');
            }
            out.push_str(&escape_csv_cell(val));
        }
        if row_idx + 1 < all_rows.len() {
            out.push_str("\r\n");
        }
    }

    out
}

/// Escapes a CSV cell value according to RFC 4180 rules.
fn escape_csv_cell(val: &str) -> String {
    let needs_quotes = val.contains(',') || val.contains('"') || val.contains('\n') || val.contains('\r');
    if needs_quotes {
        let escaped = val.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        val.to_string()
    }
}

/// Serializes a table into structured JSON.
pub fn export_to_json(table: &DetectedTable) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"table_index\": {},\n", table.table_idx));
    out.push_str(&format!("  \"page_number\": {},\n", table.page_number));
    out.push_str(&format!(
        "  \"bbox\": [{:.2}, {:.2}, {:.2}, {:.2}],\n",
        table.bbox.min_x, table.bbox.min_y, table.bbox.max_x, table.bbox.max_y
    ));
    out.push_str(&format!("  \"row_count\": {},\n", table.row_count));
    out.push_str(&format!("  \"col_count\": {},\n", table.col_count));

    // Headers
    out.push_str("  \"headers\": [");
    for (i, h) in table.headers.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("\"{}\"", escape_json_str(h)));
    }
    out.push_str("],\n");

    // Rows
    out.push_str("  \"rows\": [\n");
    for (r_idx, row) in table.rows.iter().enumerate() {
        out.push_str("    [");
        for (c_idx, val) in row.iter().enumerate() {
            if c_idx > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("\"{}\"", escape_json_str(val)));
        }
        out.push(']');
        if r_idx + 1 < table.rows.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ],\n");

    // Cells
    out.push_str("  \"cells\": [\n");
    for (i, cell) in table.cells.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"row\": {},\n", cell.row_idx));
        out.push_str(&format!("      \"col\": {},\n", cell.col_idx));
        out.push_str(&format!("      \"row_span\": {},\n", cell.row_span));
        out.push_str(&format!("      \"col_span\": {},\n", cell.col_span));
        out.push_str(&format!("      \"is_header\": {},\n", cell.is_header));
        out.push_str(&format!(
            "      \"bbox\": [{:.2}, {:.2}, {:.2}, {:.2}],\n",
            cell.bbox.min_x, cell.bbox.min_y, cell.bbox.max_x, cell.bbox.max_y
        ));
        out.push_str(&format!("      \"text\": \"{}\"\n", escape_json_str(&cell.text)));
        out.push_str("    }");
        if i + 1 < table.cells.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n");
    out.push('}');
    out
}

/// Escapes a string for JSON output.
fn escape_json_str(val: &str) -> String {
    let mut s = String::with_capacity(val.len());
    for c in val.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            '\x08' => s.push_str("\\b"),
            '\x0C' => s.push_str("\\f"),
            c if (c as u32) < 0x20 => s.push_str(&format!("\\u{:04x}", c as u32)),
            c => s.push(c),
        }
    }
    s
}

/// Serializes a table into GitHub Flavored Markdown.
pub fn export_to_markdown(table: &DetectedTable) -> String {
    let mut out = String::new();

    if table.col_count == 0 {
        return out;
    }

    // Headers row
    let headers: Vec<String> = if !table.headers.is_empty() {
        table.headers.clone()
    } else {
        (0..table.col_count).map(|i| format!("Col {}", i + 1)).collect()
    };

    out.push('|');
    for h in &headers {
        out.push(' ');
        out.push_str(&escape_markdown_cell(h));
        out.push_str(" |");
    }
    out.push('\n');

    // Separator row
    out.push('|');
    for _ in 0..headers.len() {
        out.push_str(" :--- |");
    }
    out.push('\n');

    // Data rows
    for row in &table.rows {
        out.push('|');
        for c_idx in 0..headers.len() {
            let val = row.get(c_idx).map(|s| s.as_str()).unwrap_or("");
            out.push(' ');
            out.push_str(&escape_markdown_cell(val));
            out.push_str(" |");
        }
        out.push('\n');
    }

    out
}

/// Escapes characters that break Markdown table syntax.
fn escape_markdown_cell(val: &str) -> String {
    val.replace('|', "\\|")
        .replace('\r', "")
        .replace('\n', "<br/>")
        .trim()
        .to_string()
}

/// Serializes a table into semantic HTML.
pub fn export_to_html(table: &DetectedTable) -> String {
    let mut out = String::new();
    out.push_str("<table class=\"pdf-extracted-table\" border=\"1\">\n");

    if !table.headers.is_empty() {
        out.push_str("  <thead>\n    <tr>\n");
        for h in &table.headers {
            out.push_str(&format!("      <th>{}</th>\n", escape_html(h)));
        }
        out.push_str("    </tr>\n  </thead>\n");
    }

    out.push_str("  <tbody>\n");
    for row in &table.rows {
        out.push_str("    <tr>\n");
        for cell_val in row {
            out.push_str(&format!("      <td>{}</td>\n", escape_html(cell_val)));
        }
        out.push_str("    </tr>\n");
    }
    out.push_str("  </tbody>\n");
    out.push_str("</table>");

    out
}

/// Escapes HTML special entities.
fn escape_html(val: &str) -> String {
    val.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
