//! The compact local description of a schema that goes into AI prompts.
//!
//! Built from the driver's catalog queries, stored per connection, and sent
//! one table per line, e.g.
//! `orders(id int pk, customer_id int fk→customers.id, status text?)`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::db::{SchemaDetails, SchemaSnapshot};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexColumn {
    pub name: String,
    pub type_name: String,
    pub pk: bool,
    pub nullable: bool,
    /// `table.column` (schema-qualified when needed) this column references.
    pub fk: String,
    pub comment: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexTable {
    pub schema: String,
    pub name: String,
    pub view: bool,
    pub comment: String,
    pub rows: Option<i64>,
    pub indexes: Vec<String>,
    pub storage: String,
    pub columns: Vec<IndexColumn>,
    /// Positions in `SchemaIndex::tables` of the tables this one references.
    pub references: Vec<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SchemaIndex {
    pub server_version: String,
    pub search_path: Vec<String>,
    pub tables: Vec<IndexTable>,
    /// Changes when anything above changes; lets a refresh skip the rewrite.
    pub fingerprint: String,
    pub built_at: i64,
    pub with_samples: bool,
}

/// What a prompt gets: the rendered schema and how much of it made it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub text: String,
    pub tables_sent: usize,
    pub tables_total: usize,
}

/// A rough token count (about four characters per token). Providers tokenize
/// differently; this only has to keep the schema inside a budget.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

fn short_type(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for (long, short) in [
        ("timestamp with time zone", "timestamptz"),
        ("timestamp without time zone", "timestamp"),
        ("time with time zone", "timetz"),
        ("time without time zone", "time"),
        ("character varying", "varchar"),
        ("double precision", "float8"),
        ("boolean", "bool"),
        ("integer", "int"),
    ] {
        if let Some(rest) = lower.strip_prefix(long) {
            return format!("{short}{}", &name[long.len()..long.len() + rest.len()]);
        }
    }
    name.to_string()
}

pub fn build(
    snapshot: &SchemaSnapshot,
    details: &SchemaDetails,
    with_samples: bool,
    now_ms: i64,
) -> SchemaIndex {
    let implicit =
        |schema: &str| schema.is_empty() || snapshot.search_path.iter().any(|s| s == schema);
    let position: HashMap<(&str, &str), usize> = snapshot
        .tables
        .iter()
        .enumerate()
        .map(|(i, t)| ((t.schema.as_str(), t.name.as_str()), i))
        .collect();

    let mut tables: Vec<IndexTable> = Vec::with_capacity(snapshot.tables.len());
    for table in &snapshot.tables {
        let extra = details
            .tables
            .iter()
            .find(|d| d.schema == table.schema && d.name == table.name);
        let mut references = Vec::new();
        let mut columns: Vec<IndexColumn> = table
            .columns
            .iter()
            .map(|c| {
                let more = extra.and_then(|e| e.columns.iter().find(|d| d.name == c.name));
                IndexColumn {
                    name: c.name.clone(),
                    type_name: short_type(&c.type_name),
                    pk: c.primary_key,
                    // Unknown nullability is not worth a marker.
                    nullable: more.is_some_and(|m| m.nullable) && !c.primary_key,
                    fk: String::new(),
                    comment: more.map(|m| m.comment.clone()).unwrap_or_default(),
                    values: more.map(|m| m.values.clone()).unwrap_or_default(),
                }
            })
            .collect();
        for key in &table.foreign_keys {
            // SQLite leaves the schema empty; everything else names it.
            let schema = if key.ref_schema.is_empty() {
                table.schema.as_str()
            } else {
                key.ref_schema.as_str()
            };
            if let Some(i) = position.get(&(schema, key.ref_table.as_str())) {
                if !references.contains(i) {
                    references.push(*i);
                }
            }
            let target = if implicit(schema) {
                key.ref_table.clone()
            } else {
                format!("{schema}.{}", key.ref_table)
            };
            for (column, ref_column) in key.columns.iter().zip(&key.ref_columns) {
                if let Some(c) = columns.iter_mut().find(|c| &c.name == column) {
                    c.fk = format!("{target}.{ref_column}");
                }
            }
        }
        tables.push(IndexTable {
            schema: table.schema.clone(),
            name: table.name.clone(),
            view: table.kind == "view",
            comment: extra.map(|e| e.comment.clone()).unwrap_or_default(),
            rows: extra.and_then(|e| e.row_estimate),
            indexes: extra.map(|e| e.indexes.clone()).unwrap_or_default(),
            storage: extra.map(|e| e.storage.clone()).unwrap_or_default(),
            columns,
            references,
        });
    }

    let mut index = SchemaIndex {
        server_version: details.server_version.clone(),
        search_path: snapshot.search_path.clone(),
        tables,
        fingerprint: String::new(),
        built_at: now_ms,
        with_samples,
    };
    index.fingerprint = fingerprint(&index);
    index
}

/// FNV-1a over the structure. Row estimates are left out: they drift with
/// every write and would make every refresh look like a change.
fn fingerprint(index: &SchemaIndex) -> String {
    let mut stable = index.clone();
    stable.fingerprint.clear();
    stable.built_at = 0;
    for table in &mut stable.tables {
        table.rows = None;
    }
    let bytes = serde_json::to_vec(&stable).unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        format!("{}…", flat.chars().take(max).collect::<String>())
    } else {
        flat
    }
}

fn human_count(n: i64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 10_000 => format!("{}k", n / 1000),
        n => n.to_string(),
    }
}

impl SchemaIndex {
    fn qualified(&self, table: &IndexTable) -> String {
        if table.schema.is_empty() || self.search_path.contains(&table.schema) {
            table.name.clone()
        } else {
            format!("{}.{}", table.schema, table.name)
        }
    }

    /// One table in the compact notation.
    pub fn render(&self, table: &IndexTable) -> String {
        let columns: Vec<String> = table
            .columns
            .iter()
            .map(|c| {
                // Views and expressions can have no declared type (SQLite).
                let mut out = if c.type_name.is_empty() {
                    c.name.clone()
                } else {
                    format!(
                        "{} {}{}",
                        c.name,
                        c.type_name,
                        if c.nullable { "?" } else { "" }
                    )
                };
                if c.pk {
                    out.push_str(" pk");
                }
                if !c.fk.is_empty() {
                    out.push_str(&format!(" fk→{}", c.fk));
                }
                if !c.values.is_empty() {
                    out.push_str(&format!(" {{{}}}", c.values.join("|")));
                }
                if !c.comment.is_empty() {
                    out.push_str(&format!(" /*{}*/", one_line(&c.comment, 60)));
                }
                out
            })
            .collect();
        let mut line = format!(
            "{}{}({})",
            if table.view { "view " } else { "" },
            self.qualified(table),
            columns.join(", ")
        );
        let mut notes = Vec::new();
        if let Some(rows) = table.rows {
            notes.push(format!("~{} rows", human_count(rows)));
        }
        if !table.storage.is_empty() {
            notes.push(table.storage.clone());
        }
        if !table.indexes.is_empty() {
            notes.push(format!("idx: {}", table.indexes.join("; ")));
        }
        if !table.comment.is_empty() {
            notes.push(one_line(&table.comment, 100));
        }
        if !notes.is_empty() {
            line.push_str(&format!(" -- {}", notes.join("; ")));
        }
        line
    }

    /// The schema for a prompt. Small schemas go in whole; large ones are
    /// ranked against `question` and recent `history`, expanded along foreign
    /// keys, and cut at `budget_tokens`. Tables that do not fit are listed by
    /// name so the model at least knows they exist.
    pub fn select(&self, question: &str, history: &[String], budget_tokens: usize) -> Selection {
        let lines: Vec<String> = self.tables.iter().map(|t| self.render(t)).collect();
        let cost = |i: usize| estimate_tokens(&lines[i]) + 1;
        let total: usize = (0..lines.len()).map(cost).sum();
        if total <= budget_tokens {
            return Selection {
                text: lines.join("\n"),
                tables_sent: lines.len(),
                tables_total: lines.len(),
            };
        }

        let words = keywords(question);
        let used: HashMap<String, usize> = usage(history);
        let mut scored: Vec<(i64, usize)> = self
            .tables
            .iter()
            .enumerate()
            .map(|(i, table)| {
                let mut score = 0i64;
                let name = keywords(&table.name);
                let mut column_hits = 0i64;
                for word in &words {
                    if name.contains(word) {
                        score += 10;
                    }
                    if column_hits < 3
                        && table
                            .columns
                            .iter()
                            .any(|c| keywords(&c.name).contains(word))
                    {
                        column_hits += 1;
                    }
                    if table.comment.to_lowercase().contains(word.as_str()) {
                        score += 2;
                    }
                }
                score += column_hits * 3;
                score += used
                    .get(&table.name.to_lowercase())
                    .copied()
                    .unwrap_or(0)
                    .min(5) as i64;
                (score, i)
            })
            .collect();
        // Highest score first; schema order breaks ties so the result is stable.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

        let mut chosen: Vec<usize> = Vec::new();
        let mut seen: HashSet<usize> = HashSet::new();
        let mut spent = 0usize;
        let mut take = |i: usize, chosen: &mut Vec<usize>, spent: &mut usize| -> bool {
            if seen.contains(&i) {
                return true;
            }
            if *spent + cost(i) > budget_tokens {
                return false;
            }
            seen.insert(i);
            *spent += cost(i);
            chosen.push(i);
            true
        };
        for (score, i) in &scored {
            // Once the relevant tables are in, stop: unrelated tables are noise.
            if *score == 0 && !chosen.is_empty() {
                break;
            }
            if !take(*i, &mut chosen, &mut spent) {
                continue;
            }
            for reference in &self.tables[*i].references {
                take(*reference, &mut chosen, &mut spent);
            }
        }
        chosen.sort_unstable();

        let mut text = chosen
            .iter()
            .map(|i| lines[*i].clone())
            .collect::<Vec<_>>()
            .join("\n");
        let mut others: Vec<String> = Vec::new();
        let mut left = budget_tokens.saturating_sub(spent);
        let mut cut = false;
        for (i, table) in self.tables.iter().enumerate() {
            if chosen.binary_search(&i).is_ok() {
                continue;
            }
            let name = self.qualified(table);
            let need = estimate_tokens(&name) + 1;
            if need > left {
                cut = true;
                break;
            }
            left -= need;
            others.push(name);
        }
        if !others.is_empty() {
            text.push_str(&format!(
                "\nOther tables (columns not shown): {}{}",
                others.join(", "),
                if cut { ", …" } else { "" }
            ));
        }
        Selection {
            text,
            tables_sent: chosen.len(),
            tables_total: self.tables.len(),
        }
    }
}

const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "from", "with", "that", "this", "all", "any", "are", "was", "were", "has",
    "have", "not", "per", "each", "into", "over", "select", "where", "group", "order", "show",
    "list", "give", "get", "find", "how", "many", "much", "what", "which", "who", "when", "last",
    "first", "top", "most", "least", "than", "then", "between", "table", "tables", "column",
    "columns", "row", "rows", "query", "sql", "data", "name", "names", "count", "total",
];

/// Lowercase words of three letters or more, split on anything that is not a
/// letter or digit (so `customer_id` yields `customer`), with a plural `s`
/// dropped so "orders" meets `order`.
fn keywords(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 && !STOP_WORDS.contains(w))
        .map(|w| {
            if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") {
                w[..w.len() - 1].to_string()
            } else {
                w.to_string()
            }
        })
        .collect()
}

/// How often each identifier-like word appears in recent statements.
fn usage(history: &[String]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for statement in history {
        let words: HashSet<String> = statement
            .to_lowercase()
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect();
        for word in words {
            *counts.entry(word).or_insert(0) += 1;
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ColumnDetails, ForeignKey, SnapshotColumn, SnapshotTable, TableDetails};

    fn column(name: &str, type_name: &str, pk: bool) -> SnapshotColumn {
        SnapshotColumn {
            name: name.into(),
            type_name: type_name.into(),
            primary_key: pk,
        }
    }

    fn shop() -> (SchemaSnapshot, SchemaDetails) {
        let snapshot = SchemaSnapshot {
            search_path: vec!["public".into()],
            tables: vec![
                SnapshotTable {
                    schema: "public".into(),
                    name: "customers".into(),
                    kind: "table",
                    columns: vec![
                        column("id", "integer", true),
                        column("email", "character varying(255)", false),
                    ],
                    foreign_keys: vec![],
                },
                SnapshotTable {
                    schema: "public".into(),
                    name: "orders".into(),
                    kind: "table",
                    columns: vec![
                        column("id", "integer", true),
                        column("customer_id", "integer", false),
                        column("status", "order_status", false),
                        column("created_at", "timestamp with time zone", false),
                    ],
                    foreign_keys: vec![ForeignKey {
                        columns: vec!["customer_id".into()],
                        ref_schema: "public".into(),
                        ref_table: "customers".into(),
                        ref_columns: vec!["id".into()],
                    }],
                },
                SnapshotTable {
                    schema: "audit".into(),
                    name: "events".into(),
                    kind: "view",
                    columns: vec![column("at", "timestamp without time zone", false)],
                    foreign_keys: vec![],
                },
            ],
            functions: vec![],
        };
        let details = SchemaDetails {
            server_version: "17.2".into(),
            tables: vec![TableDetails {
                schema: "public".into(),
                name: "orders".into(),
                comment: "One row per\ncheckout".into(),
                row_estimate: Some(1_250_000),
                indexes: vec!["btree (customer_id)".into()],
                storage: String::new(),
                columns: vec![
                    ColumnDetails {
                        name: "status".into(),
                        nullable: true,
                        comment: String::new(),
                        values: vec!["new".into(), "paid".into()],
                    },
                    ColumnDetails {
                        name: "id".into(),
                        nullable: true,
                        ..Default::default()
                    },
                ],
            }],
        };
        (snapshot, details)
    }

    #[test]
    fn renders_the_compact_notation() {
        let (snapshot, details) = shop();
        let index = build(&snapshot, &details, false, 1);
        assert_eq!(
            index.render(&index.tables[1]),
            "orders(id int pk, customer_id int fk→customers.id, status order_status? {new|paid}, \
             created_at timestamptz) -- ~1.2M rows; idx: btree (customer_id); One row per checkout"
        );
        assert_eq!(
            index.render(&index.tables[0]),
            "customers(id int pk, email varchar(255))"
        );
        // Outside the search path the schema is spelled out.
        assert_eq!(
            index.render(&index.tables[2]),
            "view audit.events(at timestamp)"
        );
    }

    #[test]
    fn a_column_without_a_declared_type_is_just_its_name() {
        let (mut snapshot, details) = shop();
        snapshot.tables[2].columns[0].type_name.clear();
        let index = build(&snapshot, &details, false, 1);
        assert_eq!(index.render(&index.tables[2]), "view audit.events(at)");
    }

    #[test]
    fn a_small_schema_goes_in_whole() {
        let (snapshot, details) = shop();
        let index = build(&snapshot, &details, false, 1);
        let selection = index.select("anything", &[], 4000);
        assert_eq!((selection.tables_sent, selection.tables_total), (3, 3));
        assert_eq!(selection.text.lines().count(), 3);
    }

    fn wide(n: usize) -> SchemaIndex {
        let mut tables: Vec<SnapshotTable> = (0..n)
            .map(|i| SnapshotTable {
                schema: "public".into(),
                name: format!("filler_{i:03}"),
                kind: "table",
                columns: (0..8)
                    .map(|c| column(&format!("field_{c}"), "text", c == 0))
                    .collect(),
                foreign_keys: vec![],
            })
            .collect();
        let (shop, _) = shop();
        tables.extend(shop.tables);
        build(
            &SchemaSnapshot {
                search_path: vec!["public".into()],
                tables,
                functions: vec![],
            },
            &SchemaDetails::default(),
            false,
            1,
        )
    }

    #[test]
    fn a_large_schema_is_ranked_expanded_along_keys_and_capped() {
        let index = wide(300);
        let selection = index.select("how many orders were paid last week?", &[], 600);
        assert!(
            estimate_tokens(&selection.text) <= 600,
            "stays inside the budget"
        );
        assert!(
            selection.text.contains("orders(id int pk"),
            "the table the question names"
        );
        assert!(
            selection.text.contains("customers(id int pk"),
            "the table it references comes along"
        );
        assert!(
            !selection.text.contains("filler_000("),
            "unrelated tables are not described"
        );
        assert!(selection
            .text
            .contains("Other tables (columns not shown): "));
        assert_eq!(selection.tables_total, 303);
        assert_eq!(selection.tables_sent, 2);
    }

    #[test]
    fn history_usage_decides_when_the_question_names_nothing() {
        let index = wide(300);
        let history = vec!["SELECT * FROM filler_123 WHERE field_1 = 'x'".to_string()];
        let selection = index.select("what should I look at?", &history, 300);
        assert!(selection.text.starts_with("filler_123("));
    }

    #[test]
    fn column_names_match_too() {
        let index = wide(300);
        let selection = index.select("emails of everyone", &[], 300);
        assert!(selection.text.contains("customers(id int pk, email"));
    }

    #[test]
    fn the_fingerprint_follows_structure_not_row_counts() {
        let (snapshot, mut details) = shop();
        let a = build(&snapshot, &details, false, 1);
        details.tables[0].row_estimate = Some(9);
        let b = build(&snapshot, &details, false, 2);
        assert_eq!(a.fingerprint, b.fingerprint);
        details.tables[0].columns[0].values.push("refunded".into());
        let c = build(&snapshot, &details, false, 3);
        assert_ne!(a.fingerprint, c.fingerprint);
    }
}
