//! Inline editing: turns staged cell edits into `UPDATE` statements.
//!
//! The statements produced here are exactly what "Preview SQL" shows and what
//! Apply runs, in one transaction. Each one addresses a single row by its
//! full primary key.

use serde::Deserialize;

use crate::db::{quote_ident, quote_literal, DbError, DbKind, DbResult, EditTarget};

/// The changes to one row.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowEdit {
    /// The row's primary-key values, in the order of `EditTarget::key`.
    pub key: Vec<Option<String>>,
    /// New values by table column name; `None` sets NULL.
    pub changes: Vec<(String, Option<String>)>,
}

fn literal(kind: DbKind, value: &Option<String>) -> String {
    match value {
        // An untyped string literal: the database converts it to the column's
        // type, exactly as if the user had typed it in an UPDATE.
        Some(text) => quote_literal(kind, text),
        None => "NULL".into(),
    }
}

pub fn qualified_table(kind: DbKind, target: &EditTarget) -> String {
    if target.schema.is_empty() {
        quote_ident(kind, &target.table)
    } else {
        format!(
            "{}.{}",
            quote_ident(kind, &target.schema),
            quote_ident(kind, &target.table)
        )
    }
}

/// One `UPDATE` per edited row.
pub fn update_statements(
    kind: DbKind,
    target: &EditTarget,
    edits: &[RowEdit],
) -> DbResult<Vec<String>> {
    if target.key.is_empty() {
        return Err(DbError::other(
            "This result has no primary key, so it cannot be edited.",
        ));
    }
    let table = qualified_table(kind, target);
    let mut statements = Vec::with_capacity(edits.len());
    for edit in edits {
        if edit.key.len() != target.key.len() || edit.key.iter().any(Option::is_none) {
            return Err(DbError::other(
                "A row is missing its primary-key value and cannot be edited.",
            ));
        }
        if edit.changes.is_empty() {
            continue;
        }
        for (column, _) in &edit.changes {
            if !target.columns.iter().flatten().any(|c| c == column) {
                return Err(DbError::other(format!(
                    "{column} is not a column of {table}."
                )));
            }
        }
        let assignments: Vec<String> = edit
            .changes
            .iter()
            .map(|(column, value)| {
                format!("{} = {}", quote_ident(kind, column), literal(kind, value))
            })
            .collect();
        let conditions: Vec<String> = target
            .key
            .iter()
            .zip(&edit.key)
            .map(|(column, value)| {
                format!("{} = {}", quote_ident(kind, column), literal(kind, value))
            })
            .collect();
        statements.push(format!(
            "UPDATE {table} SET {} WHERE {}",
            assignments.join(", "),
            conditions.join(" AND ")
        ));
    }
    Ok(statements)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> EditTarget {
        EditTarget {
            schema: "public".into(),
            table: "order items".into(),
            columns: vec![
                Some("order_id".into()),
                Some("line".into()),
                Some("note".into()),
                None,
            ],
            key: vec!["order_id".into(), "line".into()],
        }
    }

    fn edit(key: &[&str], changes: &[(&str, Option<&str>)]) -> RowEdit {
        RowEdit {
            key: key.iter().map(|k| Some(k.to_string())).collect(),
            changes: changes
                .iter()
                .map(|(c, v)| (c.to_string(), v.map(str::to_string)))
                .collect(),
        }
    }

    #[test]
    fn one_update_per_row_addressed_by_its_whole_key() {
        let sql = update_statements(
            DbKind::Postgres,
            &target(),
            &[
                edit(&["7", "1"], &[("note", Some("it's fine"))]),
                edit(&["7", "2"], &[("note", None), ("line", Some("3"))]),
            ],
        )
        .unwrap();
        assert_eq!(
            sql,
            [
                "UPDATE \"public\".\"order items\" SET \"note\" = 'it''s fine' WHERE \"order_id\" = '7' AND \"line\" = '1'",
                "UPDATE \"public\".\"order items\" SET \"note\" = NULL, \"line\" = '3' WHERE \"order_id\" = '7' AND \"line\" = '2'",
            ]
        );
    }

    #[test]
    fn sqlite_tables_have_no_schema_prefix() {
        let target = EditTarget {
            schema: String::new(),
            table: "t".into(),
            columns: vec![Some("id".into()), Some("name".into())],
            key: vec!["id".into()],
        };
        let sql = update_statements(
            DbKind::Sqlite,
            &target,
            &[edit(&["1"], &[("name", Some("x"))])],
        )
        .unwrap();
        assert_eq!(sql, ["UPDATE \"t\" SET \"name\" = 'x' WHERE \"id\" = '1'"]);
    }

    #[test]
    fn rows_without_changes_produce_nothing() {
        assert!(
            update_statements(DbKind::Postgres, &target(), &[edit(&["7", "1"], &[])])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn unsafe_edits_are_refused() {
        // A column that is not part of the table (an expression in the result).
        assert!(update_statements(
            DbKind::Postgres,
            &target(),
            &[edit(&["7", "1"], &[("total", Some("1"))])]
        )
        .is_err());
        // A partial key would update more than one row.
        assert!(update_statements(
            DbKind::Postgres,
            &target(),
            &[edit(&["7"], &[("note", Some("x"))])]
        )
        .is_err());
        let mut null_key = edit(&["7", "1"], &[("note", Some("x"))]);
        null_key.key[1] = None;
        assert!(update_statements(DbKind::Postgres, &target(), &[null_key]).is_err());
        let mut keyless = target();
        keyless.key.clear();
        assert!(update_statements(
            DbKind::Postgres,
            &keyless,
            &[edit(&[], &[("note", Some("x"))])]
        )
        .is_err());
    }
}
