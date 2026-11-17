//! Export formats. Each writer is a [`RowSink`], so the same code serves
//! "export this page" (rows from the UI) and "export the full result" (rows
//! streamed straight from the database to the file).

use std::io::{self, Write};

use serde::Deserialize;

use crate::db::{quote_ident, quote_literal, Cell, Column, ColumnKind, DbKind, RowSink};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Csv,
    Tsv,
    Json,
    Ndjson,
    Sql,
    Markdown,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Json => "json",
            Self::Ndjson => "ndjson",
            Self::Sql => "sql",
            Self::Markdown => "md",
        }
    }
}

pub struct Options {
    pub format: Format,
    /// Dialect and target table name for SQL INSERT output.
    pub kind: DbKind,
    pub table: String,
}

/// Writes rows to `out` in the chosen format.
pub struct Writer<W: Write + Send> {
    out: W,
    options: Options,
    columns: Vec<Column>,
    rows: u64,
}

impl<W: Write + Send> Writer<W> {
    pub fn new(out: W, options: Options) -> Self {
        Self {
            out,
            options,
            columns: Vec::new(),
            rows: 0,
        }
    }

    pub fn into_inner(self) -> W {
        self.out
    }
}

/// RFC 4180: quote a field only when it needs it.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) || value.starts_with(' ') || value.ends_with(' ') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// TSV has no quoting: tabs, newlines and backslashes are escaped.
fn tsv_field(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', "<br>")
}

/// A cell as a JSON value: numbers and booleans keep their type when that is
/// lossless, JSON columns are embedded, everything else is a string.
fn json_value(column: &Column, cell: &Cell) -> serde_json::Value {
    use serde_json::Value;
    let Some(text) = cell else { return Value::Null };
    match column.kind {
        ColumnKind::Number => match text.parse::<i64>() {
            Ok(n) => Value::from(n),
            // Floats, and integers beyond i64, stay exact as numbers only if
            // they survive the round trip; otherwise keep the text.
            Err(_) => match text.parse::<f64>() {
                Ok(f) if f.is_finite() && f.to_string() == *text => Value::from(f),
                _ => Value::String(text.clone()),
            },
        },
        ColumnKind::Bool => match text.as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::String(text.clone()),
        },
        ColumnKind::Json => {
            serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone()))
        }
        _ => Value::String(text.clone()),
    }
}

fn sql_literal(kind: DbKind, column: &Column, cell: &Cell) -> String {
    match cell {
        None => "NULL".into(),
        Some(text) => match column.kind {
            ColumnKind::Number if text.parse::<f64>().is_ok_and(f64::is_finite) => text.clone(),
            ColumnKind::Bool if kind != DbKind::Sqlite && (text == "true" || text == "false") => {
                text.clone()
            }
            _ => quote_literal(kind, text),
        },
    }
}

impl<W: Write + Send> RowSink for Writer<W> {
    fn start(&mut self, columns: &[Column]) -> io::Result<()> {
        self.columns = columns.to_vec();
        let names = || columns.iter().map(|c| c.name.as_str());
        match self.options.format {
            Format::Csv => writeln!(
                self.out,
                "{}",
                names().map(csv_field).collect::<Vec<_>>().join(",")
            ),
            Format::Tsv => writeln!(
                self.out,
                "{}",
                names().map(tsv_field).collect::<Vec<_>>().join("\t")
            ),
            Format::Markdown => {
                writeln!(
                    self.out,
                    "| {} |",
                    names().map(markdown_cell).collect::<Vec<_>>().join(" | ")
                )?;
                let rule: Vec<&str> = columns
                    .iter()
                    .map(|c| {
                        if c.kind == ColumnKind::Number {
                            "---:"
                        } else {
                            "---"
                        }
                    })
                    .collect();
                writeln!(self.out, "| {} |", rule.join(" | "))
            }
            Format::Json => write!(self.out, "["),
            Format::Ndjson | Format::Sql => Ok(()),
        }
    }

    fn row(&mut self, cells: &[Cell]) -> io::Result<()> {
        let text = |cell: &Cell| cell.clone().unwrap_or_default();
        match self.options.format {
            Format::Csv => {
                let fields: Vec<String> = cells.iter().map(|c| csv_field(&text(c))).collect();
                writeln!(self.out, "{}", fields.join(","))?;
            }
            Format::Tsv => {
                // NULL is \N, as PostgreSQL and ClickHouse write it.
                let fields: Vec<String> = cells
                    .iter()
                    .map(|c| c.as_deref().map_or("\\N".to_string(), tsv_field))
                    .collect();
                writeln!(self.out, "{}", fields.join("\t"))?;
            }
            Format::Markdown => {
                let fields: Vec<String> = cells.iter().map(|c| markdown_cell(&text(c))).collect();
                writeln!(self.out, "| {} |", fields.join(" | "))?;
            }
            Format::Json | Format::Ndjson => {
                let object: serde_json::Map<String, serde_json::Value> = self
                    .columns
                    .iter()
                    .zip(cells)
                    .map(|(column, cell)| (column.name.clone(), json_value(column, cell)))
                    .collect();
                let line = serde_json::to_string(&object).map_err(io::Error::other)?;
                if self.options.format == Format::Json {
                    write!(
                        self.out,
                        "{}\n  {line}",
                        if self.rows == 0 { "" } else { "," }
                    )?;
                } else {
                    writeln!(self.out, "{line}")?;
                }
            }
            Format::Sql => {
                let kind = self.options.kind;
                let names: Vec<String> = self
                    .columns
                    .iter()
                    .map(|c| quote_ident(kind, &c.name))
                    .collect();
                let values: Vec<String> = self
                    .columns
                    .iter()
                    .zip(cells)
                    .map(|(column, cell)| sql_literal(kind, column, cell))
                    .collect();
                writeln!(
                    self.out,
                    "INSERT INTO {} ({}) VALUES ({});",
                    self.options.table,
                    names.join(", "),
                    values.join(", ")
                )?;
            }
        }
        self.rows += 1;
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        if self.options.format == Format::Json {
            writeln!(self.out, "{}]", if self.rows == 0 { "" } else { "\n" })?;
        }
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns() -> Vec<Column> {
        [
            ("id", ColumnKind::Number),
            ("name", ColumnKind::Text),
            ("ok", ColumnKind::Bool),
            ("doc", ColumnKind::Json),
        ]
        .into_iter()
        .map(|(name, kind)| Column {
            name: name.into(),
            type_name: String::new(),
            kind,
        })
        .collect()
    }

    fn rows() -> Vec<Vec<Cell>> {
        vec![
            vec![
                Some("1".into()),
                Some("plain".into()),
                Some("true".into()),
                Some("{\"a\":[1,2]}".into()),
            ],
            vec![
                Some("2".into()),
                Some("comma, \"quote\"\nline | pipe\ttab".into()),
                None,
                None,
            ],
        ]
    }

    fn export(format: Format) -> String {
        let mut writer = Writer::new(
            Vec::new(),
            Options {
                format,
                kind: DbKind::Postgres,
                table: "public.items".into(),
            },
        );
        writer.start(&columns()).unwrap();
        for row in rows() {
            writer.row(&row).unwrap();
        }
        writer.finish().unwrap();
        String::from_utf8(writer.into_inner()).unwrap()
    }

    #[test]
    fn csv_quotes_only_what_needs_it() {
        assert_eq!(
            export(Format::Csv),
            "id,name,ok,doc\n1,plain,true,\"{\"\"a\"\":[1,2]}\"\n2,\"comma, \"\"quote\"\"\nline | pipe\ttab\",,\n"
        );
    }

    #[test]
    fn tsv_escapes_instead_of_quoting_and_marks_null() {
        assert_eq!(
            export(Format::Tsv),
            "id\tname\tok\tdoc\n1\tplain\ttrue\t{\"a\":[1,2]}\n2\tcomma, \"quote\"\\nline | pipe\\ttab\t\\N\t\\N\n"
        );
    }

    #[test]
    fn json_keeps_types_and_is_valid() {
        let text = export(Format::Json);
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(parsed[0]["id"], 1);
        assert_eq!(parsed[0]["ok"], true);
        assert_eq!(parsed[0]["doc"]["a"][1], 2);
        assert_eq!(parsed[1]["ok"], serde_json::Value::Null);
        assert!(parsed[1]["name"]
            .as_str()
            .unwrap()
            .contains("\"quote\"\nline"));
    }

    #[test]
    fn ndjson_is_one_object_per_line() {
        let text = export(Format::Ndjson);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            serde_json::from_str::<serde_json::Value>(line).expect("each line is JSON");
        }
    }

    #[test]
    fn sql_inserts_quote_text_and_leave_numbers_bare() {
        let text = export(Format::Sql);
        let first = text.lines().next().unwrap();
        assert_eq!(
            first,
            "INSERT INTO public.items (\"id\", \"name\", \"ok\", \"doc\") VALUES (1, 'plain', true, '{\"a\":[1,2]}');"
        );
        assert!(text.contains("NULL, NULL);"));
    }

    #[test]
    fn markdown_table_escapes_pipes_and_newlines() {
        let text = export(Format::Markdown);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "| id | name | ok | doc |");
        assert_eq!(lines[1], "| ---: | --- | --- | --- |");
        assert!(lines[3].contains("line \\| pipe"));
        assert!(lines[3].contains("<br>"));
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn empty_results_are_still_well_formed() {
        for format in [Format::Csv, Format::Json, Format::Ndjson, Format::Markdown] {
            let mut writer = Writer::new(
                Vec::new(),
                Options {
                    format,
                    kind: DbKind::Sqlite,
                    table: "t".into(),
                },
            );
            writer.start(&columns()).unwrap();
            writer.finish().unwrap();
            let text = String::from_utf8(writer.into_inner()).unwrap();
            if format == Format::Json {
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&text).unwrap(),
                    serde_json::json!([])
                );
            }
        }
    }

    #[test]
    fn huge_integers_and_odd_numbers_are_not_mangled_in_json() {
        let column = Column {
            name: "n".into(),
            type_name: String::new(),
            kind: ColumnKind::Number,
        };
        let value = |text: &str| json_value(&column, &Some(text.into()));
        assert_eq!(value("42"), serde_json::json!(42));
        assert_eq!(value("1.5"), serde_json::json!(1.5));
        // Beyond what a JSON number can hold exactly: keep the digits.
        assert_eq!(
            value("123456789012345678901234567890"),
            serde_json::json!("123456789012345678901234567890")
        );
        assert_eq!(value("1.10"), serde_json::json!("1.10"));
        assert_eq!(value("NaN"), serde_json::json!("NaN"));
    }
}
