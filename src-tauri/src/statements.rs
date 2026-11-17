//! Statement handling that must be right before anything reaches a database:
//! splitting a script into statements, recognising transaction control, and
//! spotting destructive statements for the Prod guard.
//!
//! Everything here works on tokens from `sqlparser`'s dialect-aware tokenizer
//! (which understands strings, comments and PostgreSQL dollar-quoting), never
//! on a full parse: a SQL client must cope with syntax the parser has never
//! seen. Nothing in this module rewrites the user's SQL.

use serde::Serialize;
use sqlparser::dialect::{ClickHouseDialect, Dialect, PostgreSqlDialect, SQLiteDialect};
use sqlparser::keywords::Keyword;
use sqlparser::tokenizer::{Token, TokenWithSpan, Tokenizer};

use crate::db::DbKind;

fn dialect(kind: DbKind) -> Box<dyn Dialect> {
    match kind {
        DbKind::Postgres => Box::new(PostgreSqlDialect {}),
        DbKind::Clickhouse => Box::new(ClickHouseDialect {}),
        DbKind::Sqlite => Box::new(SQLiteDialect {}),
    }
}

/// One statement inside a script. Offsets are UTF-16 code units, the unit
/// JavaScript strings and the editor use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatementSpan {
    pub start: usize,
    pub end: usize,
}

/// Maps the tokenizer's (line, column) positions, which count characters, to
/// offsets in the original text.
struct Positions {
    /// For each line: byte offset of its first character.
    line_starts: Vec<usize>,
}

impl Positions {
    fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts }
    }

    /// Byte offset of a 1-based (line, column) location.
    fn byte_offset(&self, text: &str, line: u64, column: u64) -> usize {
        let Some(&start) = self.line_starts.get((line as usize).saturating_sub(1)) else {
            return text.len();
        };
        let rest = &text[start..];
        rest.char_indices()
            .nth((column as usize).saturating_sub(1))
            .map_or(text.len(), |(i, _)| start + i)
    }
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn is_trivia(token: &Token) -> bool {
    matches!(token, Token::Whitespace(_))
}

fn keyword(token: &Token) -> Option<Keyword> {
    match token {
        Token::Word(w) if w.quote_style.is_none() && w.keyword != Keyword::NoKeyword => {
            Some(w.keyword)
        }
        _ => None,
    }
}

fn tokenize(kind: DbKind, text: &str) -> Option<Vec<TokenWithSpan>> {
    let dialect = dialect(kind);
    Tokenizer::new(dialect.as_ref(), text)
        .tokenize_with_location()
        .ok()
}

/// Splits a script into statements at top-level semicolons.
///
/// Semicolons inside strings, comments, dollar-quoted bodies and compound
/// bodies (`CREATE TRIGGER … BEGIN … END`, `BEGIN ATOMIC … END`) do not split.
/// Empty statements are dropped. If the text cannot be tokenized (an
/// unterminated string, say) it is returned as a single statement so the
/// database can report the error in its own words.
pub fn split(kind: DbKind, text: &str) -> Vec<StatementSpan> {
    let whole = || {
        let trimmed = text.trim_end();
        let lead = trimmed.len() - trimmed.trim_start().len();
        if trimmed.trim_start().is_empty() {
            vec![]
        } else {
            vec![StatementSpan {
                start: utf16_len(&text[..lead]),
                end: utf16_len(trimmed),
            }]
        }
    };
    let Some(tokens) = tokenize(kind, text) else {
        return whole();
    };
    let positions = Positions::new(text);
    let offset =
        |t: &TokenWithSpan| positions.byte_offset(text, t.span.start.line, t.span.start.column);

    let mut spans = Vec::new();
    let mut start: Option<usize> = None; // byte offset of the first real token
    let mut body = Body::default();

    let push = |start: usize, end: usize, spans: &mut Vec<StatementSpan>| {
        let slice = text[start..end].trim_end();
        if !slice.is_empty() {
            spans.push(StatementSpan {
                start: utf16_len(&text[..start]),
                end: utf16_len(&text[..start + slice.len()]),
            });
        }
    };

    for token in &tokens {
        match &token.token {
            Token::EOF => break,
            t if is_trivia(t) => continue,
            Token::SemiColon if !body.inside() => {
                if let Some(s) = start.take() {
                    push(s, offset(token), &mut spans);
                }
                body = Body::default();
            }
            other => {
                if start.is_none() {
                    start = Some(offset(token));
                }
                body.observe(other);
            }
        }
    }
    if let Some(s) = start {
        push(s, text.len(), &mut spans);
    }
    spans
}

/// Tracks whether we are inside a compound body whose semicolons belong to the
/// statement: a trigger body, or a SQL-standard `BEGIN ATOMIC` function body.
#[derive(Default)]
struct Body {
    seen: usize,
    is_create: bool,
    is_trigger: bool,
    /// Depth of BEGIN…END (and CASE…END inside it).
    depth: usize,
    previous: Option<Keyword>,
}

impl Body {
    fn inside(&self) -> bool {
        self.depth > 0
    }

    fn observe(&mut self, token: &Token) {
        let kw = keyword(token);
        if self.seen == 0 {
            self.is_create = kw == Some(Keyword::CREATE);
        }
        self.seen += 1;
        if self.is_create && kw == Some(Keyword::TRIGGER) && self.depth == 0 {
            self.is_trigger = true;
        }
        match kw {
            Some(Keyword::BEGIN) if self.depth == 0 && self.is_trigger => self.depth = 1,
            Some(Keyword::ATOMIC) if self.depth == 0 && self.previous == Some(Keyword::BEGIN) => {
                self.depth = 1
            }
            Some(Keyword::CASE) if self.depth > 0 => self.depth += 1,
            Some(Keyword::END) if self.depth > 0 => self.depth -= 1,
            _ => {}
        }
        self.previous = kw;
    }
}

/// How a statement relates to the session's transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxControl {
    Begin,
    Commit,
    Rollback,
    /// Any other statement.
    None,
}

/// Recognises `BEGIN` / `START TRANSACTION`, `COMMIT` / `END`, and
/// `ROLLBACK` / `ABORT`. Variants that do not end the transaction
/// (`ROLLBACK TO SAVEPOINT`, `COMMIT PREPARED`, …) are `None`.
pub fn tx_control(kind: DbKind, sql: &str) -> TxControl {
    let Some(tokens) = tokenize(kind, sql) else {
        return TxControl::None;
    };
    let mut words = tokens
        .iter()
        .filter(|t| !is_trivia(&t.token) && t.token != Token::EOF)
        .map(|t| match &t.token {
            Token::Word(w) if w.quote_style.is_none() => w.value.to_ascii_uppercase(),
            _ => String::new(),
        });
    let first = words.next().unwrap_or_default();
    let second = words.next().unwrap_or_default();
    match (first.as_str(), second.as_str()) {
        ("BEGIN", _) | ("START", "TRANSACTION") => TxControl::Begin,
        ("COMMIT" | "END", "PREPARED") => TxControl::None,
        ("COMMIT" | "END", _) => TxControl::Commit,
        ("ROLLBACK" | "ABORT", "TO" | "PREPARED") => TxControl::None,
        ("ROLLBACK" | "ABORT", _) => TxControl::Rollback,
        _ => TxControl::None,
    }
}

/// One statement-shaped scope: the statement itself, or a parenthesised
/// sub-statement such as the body of a data-modifying CTE.
#[derive(Default)]
struct Scope {
    verb: Option<Keyword>,
    /// Still skipping a prefix (`WITH …`, `EXPLAIN ANALYZE …`) before the verb.
    after_with: bool,
    /// Saw `EXPLAIN`: its option list `(ANALYZE, …)` is not an expression.
    explaining: bool,
    has_where: bool,
    /// Something other than a keyword came first: not a statement.
    closed: bool,
}

impl Scope {
    fn observe(&mut self, kw: Keyword, top_level: bool) {
        match self.verb {
            Some(_) => {
                if kw == Keyword::WHERE {
                    self.has_where = true;
                }
            }
            None if self.closed => {}
            None if kw == Keyword::WITH => self.after_with = true,
            // `EXPLAIN ANALYZE DELETE …` really runs the DELETE.
            None if top_level
                && matches!(kw, Keyword::EXPLAIN | Keyword::ANALYZE | Keyword::VERBOSE) =>
            {
                self.explaining = true;
            }
            None if self.after_with => {
                // Skip the CTE list; the main verb follows it.
                if matches!(
                    kw,
                    Keyword::SELECT
                        | Keyword::INSERT
                        | Keyword::UPDATE
                        | Keyword::DELETE
                        | Keyword::MERGE
                ) {
                    self.verb = Some(kw);
                }
            }
            // Inside parentheses only UPDATE and DELETE can be statements
            // (`WITH x AS (DELETE …)`); anything else there is an expression,
            // a column list, and so on.
            None if !top_level && !matches!(kw, Keyword::UPDATE | Keyword::DELETE) => {
                self.closed = true
            }
            None => self.verb = Some(kw),
        }
    }

    fn reason(&self) -> Option<&'static str> {
        match self.verb? {
            Keyword::DROP => Some("DROP removes objects permanently."),
            Keyword::TRUNCATE => Some("TRUNCATE deletes every row."),
            Keyword::ALTER => Some("ALTER changes the structure of live objects."),
            Keyword::UPDATE if !self.has_where => Some("UPDATE without WHERE changes every row."),
            Keyword::DELETE if !self.has_where => Some("DELETE without WHERE deletes every row."),
            _ => None,
        }
    }
}

/// Why the Prod guard wants a confirmation, or `None` for a statement that is
/// not destructive: `DROP`, `TRUNCATE`, `ALTER`, and `UPDATE` / `DELETE`
/// without a `WHERE`. That includes an `UPDATE` / `DELETE` hidden in a CTE
/// (`WITH x AS (DELETE FROM t RETURNING *) SELECT …`) and one that
/// `EXPLAIN ANALYZE` would execute.
pub fn destructive_reason(kind: DbKind, sql: &str) -> Option<&'static str> {
    let tokens = tokenize(kind, sql)?;
    // One scope per open parenthesis, innermost last.
    let mut scopes = vec![Scope::default()];
    let mut found = None;

    for token in &tokens {
        match &token.token {
            Token::EOF => break,
            t if is_trivia(t) => {}
            Token::LParen => {
                // A parenthesis as the first thing in a scope is not a verb.
                if let Some(scope) = scopes.last_mut() {
                    if scope.verb.is_none() && !scope.after_with && !scope.explaining {
                        scope.closed = true;
                    }
                }
                scopes.push(Scope::default());
            }
            Token::RParen => {
                if scopes.len() > 1 {
                    let scope = scopes.pop().expect("non-empty");
                    found = found.or(scope.reason());
                }
            }
            t => {
                let top_level = scopes.len() == 1;
                let scope = scopes.last_mut().expect("non-empty");
                match keyword(t) {
                    Some(kw) => scope.observe(kw, top_level),
                    None if scope.verb.is_none() && !scope.after_with => scope.closed = true,
                    None => {}
                }
            }
        }
    }
    // The statement's own verb is the most useful reason to show.
    scopes.first().and_then(Scope::reason).or(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use DbKind::*;

    fn parts(kind: DbKind, text: &str) -> Vec<String> {
        let units: Vec<u16> = text.encode_utf16().collect();
        split(kind, text)
            .into_iter()
            .map(|s| String::from_utf16(&units[s.start..s.end]).unwrap())
            .collect()
    }

    #[test]
    fn splits_on_top_level_semicolons() {
        assert_eq!(
            parts(Postgres, "select 1; select 2 ;\n\nselect 3"),
            ["select 1", "select 2", "select 3"]
        );
    }

    #[test]
    fn drops_empty_statements_and_trailing_whitespace() {
        assert_eq!(parts(Postgres, " ;; select 1 ;  ; "), ["select 1"]);
        assert!(parts(Postgres, "   \n ").is_empty());
        assert!(parts(Postgres, "").is_empty());
    }

    #[test]
    fn semicolons_in_strings_and_comments_do_not_split() {
        assert_eq!(
            parts(Postgres, "select ';' as a; -- ; not here\nselect /* ; */ 2"),
            ["select ';' as a", "select /* ; */ 2"]
        );
        assert_eq!(parts(Sqlite, r#"select "a;b" from t"#).len(), 1);
    }

    #[test]
    fn dollar_quoted_bodies_stay_whole() {
        let sql = "create function f() returns int as $body$\nbegin\n  return 1;\nend;\n$body$ language plpgsql;\nselect f()";
        let got = parts(Postgres, sql);
        assert_eq!(got.len(), 2);
        assert!(got[0].ends_with("language plpgsql"));
        assert_eq!(got[1], "select f()");
    }

    #[test]
    fn trigger_bodies_stay_whole() {
        let sql = "create trigger t after insert on a begin\n  update b set n = case when 1 then 2 else 3 end;\n  delete from c;\nend;\nselect 1;";
        let got = parts(Sqlite, sql);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[0].ends_with("end"));
    }

    #[test]
    fn begin_atomic_function_bodies_stay_whole() {
        let sql = "create function f() returns int begin atomic select 1; select 2; end; select 3";
        assert_eq!(parts(Postgres, sql).len(), 2);
    }

    #[test]
    fn a_plain_begin_is_its_own_statement() {
        assert_eq!(
            parts(Postgres, "begin; update t set a = 1; commit;"),
            ["begin", "update t set a = 1", "commit"]
        );
    }

    #[test]
    fn offsets_are_utf16_code_units() {
        // "é" is 1 unit, "😀" is 2.
        let text = "select 'é😀'; select 2";
        let spans = split(Postgres, text);
        assert_eq!(spans[0], StatementSpan { start: 0, end: 12 });
        assert_eq!(spans[1], StatementSpan { start: 14, end: 22 });
    }

    #[test]
    fn untokenizable_text_is_one_statement() {
        assert_eq!(
            parts(Postgres, "  select 'oops; select 2  "),
            ["select 'oops; select 2"]
        );
    }

    #[test]
    fn clickhouse_specific_syntax_survives() {
        let sql = "select arrayJoin([1,2]) as x format JSON settings max_threads=1; select 2";
        assert_eq!(parts(Clickhouse, sql).len(), 2);
    }

    #[test]
    fn recognises_transaction_control() {
        for (sql, want) in [
            ("BEGIN", TxControl::Begin),
            (
                "  begin transaction isolation level serializable",
                TxControl::Begin,
            ),
            ("start transaction", TxControl::Begin),
            ("-- note\ncommit", TxControl::Commit),
            ("END", TxControl::Commit),
            ("rollback", TxControl::Rollback),
            ("ABORT", TxControl::Rollback),
            ("rollback to savepoint a", TxControl::None),
            ("commit prepared 'x'", TxControl::None),
            ("select 'begin'", TxControl::None),
            ("start_something()", TxControl::None),
        ] {
            assert_eq!(tx_control(Postgres, sql), want, "{sql}");
        }
        assert_eq!(tx_control(Sqlite, "begin immediate"), TxControl::Begin);
    }

    #[test]
    fn destructive_statements_are_flagged() {
        for sql in [
            "drop table users",
            "DROP DATABASE prod",
            "truncate orders",
            "alter table t drop column c",
            "update users set admin = true",
            "delete from users",
            "/* cleanup */ delete from users",
            "with old as (select id from t where x) delete from t",
            "update t set a = (select b from u where u.id = 1)",
            "alter table t delete where 1", // ClickHouse mutation
            // Hidden inside a CTE, or executed by EXPLAIN ANALYZE.
            "with gone as (delete from users returning *) select count(*) from gone",
            "with a as (select 1), b as (update users set admin = true returning id) select * from b",
            "explain analyze delete from users",
            "EXPLAIN (ANALYZE, BUFFERS) UPDATE users SET admin = true",
            "explain analyze drop table users",
        ] {
            assert!(destructive_reason(Postgres, sql).is_some(), "{sql}");
        }
    }

    #[test]
    fn safe_statements_are_not_flagged() {
        for sql in [
            "select * from users",
            "update users set admin = true where id = 1",
            "delete from users where id in (select id from banned)",
            "insert into t values (1)",
            "with x as (select 1) select * from x",
            "select 'drop table users'",
            "-- drop table users\nselect 1",
            "create table dropped (truncate int)",
            "explain select 1",
            "with gone as (delete from users where id = 1 returning *) select * from gone",
            "explain analyze delete from users where id = 1",
            "select (select count(*) from t), coalesce(update_count, 0) from stats",
            "insert into t (id) select id from u on conflict (id) do update set id = excluded.id",
            "select * from t where id in (select id from deleted)",
        ] {
            assert_eq!(destructive_reason(Postgres, sql), None, "{sql}");
        }
    }
}
