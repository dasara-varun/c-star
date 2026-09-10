use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Span {
    pub start_line: usize, // 1-indexed
    pub start_col: usize,  // 1-indexed
    pub end_line: usize,   // 1-indexed
    pub end_col: usize,    // 1-indexed
}

impl Span {
    pub fn new(start_line: usize, start_col: usize, end_line: usize, end_col: usize) -> Self {
        Self {
            start_line,
            start_col,
            end_line,
            end_col,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Note {
    pub message: String,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SuggestedFix {
    pub description: String,
    pub replacement: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String, // "error" or "warning"
    pub message: String,
    pub file: String,
    pub span: Span,
    pub notes: Vec<Note>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_fix: Option<SuggestedFix>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsReport {
    pub version: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl DiagnosticsReport {
    pub fn new(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            version: "1".to_string(),
            diagnostics,
        }
    }

    /// Renders the report to a human-readable string.
    pub fn render_human(&self, source_cache: &std::collections::HashMap<String, String>) -> String {
        let mut output = String::new();
        for diag in &self.diagnostics {
            let severity_upper = diag.severity.to_uppercase();
            output.push_str(&format!(
                "{} [{}]: {}\n",
                severity_upper, diag.code, diag.message
            ));
            output.push_str(&format!(
                "  --> {}:{}:{}\n",
                diag.file, diag.span.start_line, diag.span.start_col
            ));

            // Print the source line if available
            if let Some(source) = source_cache.get(&diag.file) {
                let lines: Vec<&str> = source.lines().collect();
                if diag.span.start_line > 0 && diag.span.start_line <= lines.len() {
                    let line_content = lines[diag.span.start_line - 1];
                    let line_num_str = format!(" {} | ", diag.span.start_line);
                    output.push_str(&line_num_str);
                    output.push_str(line_content);
                    output.push('\n');

                    // Print pointer caret
                    let padding = " ".repeat(line_num_str.len() - 3);
                    output.push_str(&padding);
                    output.push_str(" | ");
                    
                    let start_col = diag.span.start_col.saturating_sub(1);
                    output.push_str(&" ".repeat(start_col));

                    let span_len = if diag.span.start_line == diag.span.end_line {
                        diag.span.end_col.saturating_sub(diag.span.start_col).max(1)
                    } else {
                        1
                    };
                    output.push_str(&"^".repeat(span_len));
                    output.push('\n');
                }
            }

            for note in &diag.notes {
                output.push_str(&format!("  = note: {}\n", note.message));
                if let Some(note_span) = note.span {
                    output.push_str(&format!(
                        "    at {}:{}:{}\n",
                        diag.file, note_span.start_line, note_span.start_col
                    ));
                }
            }

            if let Some(ref fix) = diag.suggested_fix {
                output.push_str(&format!(
                    "  = help: {} -> replace with `{}`\n",
                    fix.description, fix.replacement
                ));
            }
            output.push('\n');
        }
        output
    }
}
