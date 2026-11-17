//! PostgreSQL driver tests against a real server.
//!
//!     docker compose up -d --wait
//!     cargo test --features integration --test postgres
//!
//! Server location comes from `ROWBOAT_TEST_PG_HOST` / `_PORT` (default
//! 127.0.0.1:5432, the docker-compose service).
//!
//! The TLS tests additionally need `ROWBOAT_TEST_PG_TLS_DIR`, a directory
//! holding `ca.crt`, `client.crt` and `client.key`, and a server that
//! - has `ssl = on` with a certificate for `localhost` and `127.0.0.1` signed
//!   by that CA, and `ssl_ca_file` set to it;
//! - has a role `certuser` whose `pg_hba.conf` line is `hostssl … cert`.
//!
//! Without the variable they are skipped with a message.
#![cfg(feature = "integration")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use rowboat_lib::connections::{ConnectionConfig, TlsMode};
use rowboat_lib::db::postgres::PgSession;
use rowboat_lib::db::{ConnectParams, DbKind, ErrorKind, ExecOptions, QueryResult, Session};

const ALL: ExecOptions = ExecOptions { row_limit: 0 };

fn host() -> String {
    std::env::var("ROWBOAT_TEST_PG_HOST").unwrap_or_else(|_| "127.0.0.1".into())
}

fn port() -> u16 {
    std::env::var("ROWBOAT_TEST_PG_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(5432)
}

fn params() -> ConnectParams {
    ConnectParams {
        config: ConnectionConfig {
            name: "test".into(),
            kind: DbKind::Postgres,
            host: host(),
            port: port(),
            database: "rowboat_test".into(),
            user: "rowboat".into(),
            use_pgpass: false,
            ..Default::default()
        },
        password: Some("rowboat".into()),
        client_id: "rowboat-test".into(),
        connect_timeout: Duration::from_secs(10),
        keepalive: Duration::from_secs(30),
        auto_commit: true,
        auto_reconnect: true,
        via: None,
    }
}

async fn connect() -> PgSession {
    PgSession::connect(params())
        .await
        .expect("connect to the test server")
}

fn scalar(result: &QueryResult) -> String {
    result.rows[0][0].clone().unwrap_or_else(|| "NULL".into())
}

/// A table name unique to one test, so tests can run in parallel.
fn table(name: &str) -> String {
    format!("it_{name}_{}", std::process::id())
}

#[tokio::test]
async fn client_id_is_sent_as_application_name() {
    let mut p = params();
    p.client_id = "rowboat-dominic".into();
    let s = PgSession::connect(p).await.unwrap();
    let r = s
        .execute(
            "SELECT application_name FROM pg_stat_activity WHERE pid = pg_backend_pid()",
            ALL,
        )
        .await
        .unwrap();
    assert_eq!(scalar(&r), "rowboat-dominic");
    assert!(s.server_version().starts_with("PostgreSQL "));
}

#[tokio::test]
async fn row_limit_fetches_one_page_of_a_huge_result() {
    let s = connect().await;
    let started = Instant::now();
    // 10 million rows if read to the end.
    let r = s
        .execute(
            "SELECT * FROM generate_series(1, 10000000) AS n",
            ExecOptions { row_limit: 1000 },
        )
        .await
        .unwrap();
    assert_eq!(r.rows.len(), 1000);
    assert!(r.more_available);
    assert_eq!(r.rows[999][0].as_deref(), Some("1000"));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "took {:?}: the rest was fetched",
        started.elapsed()
    );
    // The user's SQL reached the server untouched.
    assert!(!r.in_transaction);
}

#[tokio::test]
async fn exactly_the_limit_is_not_reported_as_more() {
    let s = connect().await;
    let r = s
        .execute(
            "SELECT generate_series(1, 50)",
            ExecOptions { row_limit: 50 },
        )
        .await
        .unwrap();
    assert_eq!(r.rows.len(), 50);
    assert!(!r.more_available);
    let none = s
        .execute("SELECT 1 WHERE false", ExecOptions { row_limit: 50 })
        .await
        .unwrap();
    assert!(none.rows.is_empty() && !none.more_available && none.columns.len() == 1);
}

/// Every supported type must render exactly as the server's own `::text`.
#[tokio::test]
async fn values_render_like_the_servers_text_output() {
    let s = connect().await;
    s.execute("SET TIME ZONE 'UTC'", ALL).await.unwrap();
    let expressions = [
        "42::int2",
        "-7::int4",
        "9007199254740993::int8",
        "1.5::float4",
        "0.1::float8",
        "1e20::float8",
        "1.5e-7::float8",
        "'NaN'::float8",
        "'-Infinity'::float8",
        "123456789.012345678900::numeric",
        "-0.0005::numeric",
        "0::numeric(10,3)",
        "1e30::numeric",
        "'NaN'::numeric",
        "12345678901234567890123456789::numeric",
        "'hello'::text",
        "'x'::varchar(5)",
        "'pg_class'::name",
        "'\\xdeadbeef'::bytea",
        "'{\"a\": [1, 2]}'::json",
        "'{\"a\": [1, 2]}'::jsonb",
        "'a1b2c3d4-e5f6-4789-8abc-def012345678'::uuid",
        "'2024-02-29'::date",
        "'0044-03-15 BC'::date",
        "'infinity'::date",
        "'13:14:15.123456'::time",
        "'13:14:15+05:30'::timetz",
        "'2024-02-29 13:14:15.5'::timestamp",
        "'2024-02-29 13:14:15+02'::timestamptz",
        "'infinity'::timestamptz",
        "'1 year 2 months 3 days 04:05:06.789'::interval",
        "'-1 day 2 hours'::interval",
        "'0'::interval",
        "'192.168.1.0/24'::inet",
        "'10.0.0.0/8'::cidr",
        "'2001:db8::/32'::cidr",
        "'08:00:2b:01:02:03'::macaddr",
        "B'1011'::bit(4)",
        "B'101'::varbit",
        "'16/B374D848'::pg_lsn",
        "'(1,2)'::point",
        "ARRAY[1, 2, 3]",
        "ARRAY[['a', 'b'], ['c', 'd']]",
        "ARRAY['with space', 'q\"uote', NULL, '']",
        "ARRAY[1.5, NULL]::numeric[]",
        "'{}'::int[]",
        "ARRAY['2024-01-01'::date]",
        "int4range(1, 10)",
        "'[2024-01-01,2024-02-01)'::daterange",
        "numrange(NULL, 5.5)",
        "'empty'::int4range",
        "'{[1,3), [5,8)}'::int4multirange",
        "ROW(1, 'two words', NULL)::record_probe",
        "'happy'::mood",
        "'42'::positive_int",
        "$1.50::text::money".trim_start_matches('$'),
    ];
    // Types the expressions above refer to.
    for setup in [
        "DROP TYPE IF EXISTS record_probe, mood CASCADE",
        "DROP DOMAIN IF EXISTS positive_int",
        "CREATE TYPE record_probe AS (a int, b text, c date)",
        "CREATE TYPE mood AS ENUM ('sad', 'happy')",
        "CREATE DOMAIN positive_int AS int CHECK (VALUE > 0)",
    ] {
        s.execute(setup, ALL).await.unwrap();
    }
    for expression in expressions {
        let r = s
            .execute(&format!("SELECT ({expression}), ({expression})::text"), ALL)
            .await
            .unwrap_or_else(|e| panic!("{expression}: {e:?}"));
        assert_eq!(r.rows[0][0], r.rows[0][1], "{expression}");
    }
    // `inet::text` always appends the netmask; psql's display form does not.
    let hosts = s
        .execute(
            "SELECT '192.168.1.10'::inet, '::1'::inet, 'padded'::char(8)",
            ALL,
        )
        .await
        .unwrap();
    assert_eq!(
        hosts.rows[0],
        vec![
            Some("192.168.1.10".into()),
            Some("::1".into()),
            Some("padded  ".into())
        ]
    );
    let null = s
        .execute("SELECT NULL::int, NULL::text[]", ALL)
        .await
        .unwrap();
    assert_eq!(null.rows[0], vec![None, None]);
    let boolean = s.execute("SELECT true, false", ALL).await.unwrap();
    assert_eq!(
        boolean.rows[0],
        vec![Some("true".into()), Some("false".into())]
    );
}

#[tokio::test]
async fn errors_carry_kind_position_and_hint() {
    let s = connect().await;

    let syntax = s.execute("SELECT 1 FROMM pg_class", ALL).await.unwrap_err();
    assert_eq!(syntax.kind, ErrorKind::Syntax);
    assert_eq!(syntax.code.as_deref(), Some("42601"));
    assert_eq!(syntax.position, Some(16));

    let schema = s
        .execute("SELECT * FROM no_such_table", ALL)
        .await
        .unwrap_err();
    assert_eq!(schema.kind, ErrorKind::Schema);
    assert_eq!(schema.position, Some(15));

    let column = s
        .execute("SELECT relnam FROM pg_class", ALL)
        .await
        .unwrap_err();
    assert_eq!(column.kind, ErrorKind::Schema);
    assert!(column.hint.unwrap().contains("relname"));

    // An error does not poison the session.
    assert_eq!(scalar(&s.execute("SELECT 1", ALL).await.unwrap()), "1");
}

#[tokio::test]
async fn wrong_password_and_unreachable_server_are_distinguished() {
    let mut wrong = params();
    wrong.password = Some("nope".into());
    let error = PgSession::connect(wrong).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Permission, "{error:?}");

    let mut nowhere = params();
    nowhere.config.port = 1; // nothing listens here
    nowhere.connect_timeout = Duration::from_secs(3);
    let error = PgSession::connect(nowhere).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Connection, "{error:?}");
}

#[tokio::test]
async fn cancel_stops_a_sixty_second_query_within_a_second() {
    let s = Arc::new(connect().await);
    let runner = s.clone();
    let task = tokio::spawn(async move { runner.execute("SELECT pg_sleep(60)", ALL).await });
    tokio::time::sleep(Duration::from_millis(300)).await;

    let cancelled_at = Instant::now();
    s.cancel().await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(1),
        "{:?}",
        cancelled_at.elapsed()
    );

    assert_eq!(scalar(&s.execute("SELECT 2", ALL).await.unwrap()), "2");
}

#[tokio::test]
async fn auto_commit_commits_each_statement_including_returning() {
    let s = connect().await;
    let peer = connect().await;
    let t = table("auto");
    s.execute(
        &format!("CREATE TABLE {t} (id int PRIMARY KEY, v text)"),
        ALL,
    )
    .await
    .unwrap();

    let inserted = s
        .execute(
            &format!("INSERT INTO {t} VALUES (1, 'a'), (2, 'b') RETURNING id"),
            ALL,
        )
        .await
        .unwrap();
    assert_eq!(inserted.rows.len(), 2);
    assert!(!inserted.in_transaction);
    assert_eq!(
        scalar(
            &peer
                .execute(&format!("SELECT count(*) FROM {t}"), ALL)
                .await
                .unwrap()
        ),
        "2"
    );

    let updated = s
        .execute(&format!("UPDATE {t} SET v = 'z'"), ALL)
        .await
        .unwrap();
    assert_eq!(updated.rows_affected, Some(2));
    assert!(updated.columns.is_empty());

    s.execute(&format!("DROP TABLE {t}"), ALL).await.unwrap();
}

#[tokio::test]
async fn statements_that_cannot_run_in_a_transaction_still_work() {
    let s = connect().await;
    let t = table("vacuum");
    s.execute(&format!("CREATE TABLE {t} (id int)"), ALL)
        .await
        .unwrap();
    s.execute(&format!("VACUUM {t}"), ALL).await.unwrap();
    s.execute(&format!("DROP TABLE {t}"), ALL).await.unwrap();
}

#[tokio::test]
async fn manual_commit_holds_changes_until_commit() {
    let s = connect().await;
    let peer = connect().await;
    let t = table("manual");
    s.execute(&format!("CREATE TABLE {t} (id int)"), ALL)
        .await
        .unwrap();
    let count = |r: QueryResult| scalar(&r);

    s.set_auto_commit(false);
    let r = s
        .execute(&format!("INSERT INTO {t} VALUES (1)"), ALL)
        .await
        .unwrap();
    assert!(r.in_transaction && s.in_transaction());
    assert_eq!(
        count(
            peer.execute(&format!("SELECT count(*) FROM {t}"), ALL)
                .await
                .unwrap()
        ),
        "0"
    );

    s.rollback().await.unwrap();
    assert!(!s.in_transaction());
    assert_eq!(
        count(
            peer.execute(&format!("SELECT count(*) FROM {t}"), ALL)
                .await
                .unwrap()
        ),
        "0"
    );

    s.execute(&format!("INSERT INTO {t} VALUES (2)"), ALL)
        .await
        .unwrap();
    s.commit().await.unwrap();
    assert_eq!(
        count(
            peer.execute(&format!("SELECT count(*) FROM {t}"), ALL)
                .await
                .unwrap()
        ),
        "1"
    );

    s.set_auto_commit(true);
    s.execute(&format!("DROP TABLE {t}"), ALL).await.unwrap();
}

#[tokio::test]
async fn typed_begin_and_commit_control_the_transaction() {
    let s = connect().await;
    let peer = connect().await;
    let t = table("typed");
    s.execute(&format!("CREATE TABLE {t} (id int)"), ALL)
        .await
        .unwrap();

    let begin = s
        .execute("BEGIN ISOLATION LEVEL REPEATABLE READ", ALL)
        .await
        .unwrap();
    assert!(begin.in_transaction);
    let level = s.execute("SHOW transaction_isolation", ALL).await.unwrap();
    assert_eq!(scalar(&level), "repeatable read");
    s.execute(&format!("INSERT INTO {t} VALUES (1)"), ALL)
        .await
        .unwrap();
    assert_eq!(
        scalar(
            &peer
                .execute(&format!("SELECT count(*) FROM {t}"), ALL)
                .await
                .unwrap()
        ),
        "0"
    );
    let commit = s.execute("COMMIT", ALL).await.unwrap();
    assert!(!commit.in_transaction && !s.in_transaction());
    assert_eq!(
        scalar(
            &peer
                .execute(&format!("SELECT count(*) FROM {t}"), ALL)
                .await
                .unwrap()
        ),
        "1"
    );

    // An error inside a transaction leaves it open (and failed) until rollback.
    s.execute("BEGIN", ALL).await.unwrap();
    s.execute("SELECT * FROM nope", ALL).await.unwrap_err();
    assert!(s.in_transaction());
    let aborted = s.execute("SELECT 1", ALL).await.unwrap_err();
    assert_eq!(aborted.code.as_deref(), Some("25P02"));
    s.execute("ROLLBACK", ALL).await.unwrap();
    assert_eq!(scalar(&s.execute("SELECT 1", ALL).await.unwrap()), "1");

    let stray = s.execute("COMMIT", ALL).await.unwrap();
    assert_eq!(stray.notices, ["There is no transaction in progress."]);
    s.execute(&format!("DROP TABLE {t}"), ALL).await.unwrap();
}

#[tokio::test]
async fn read_only_connections_refuse_writes() {
    let mut p = params();
    p.config.read_only = true;
    let s = PgSession::connect(p).await.unwrap();
    assert_eq!(scalar(&s.execute("SELECT 1", ALL).await.unwrap()), "1");
    let error = s
        .execute("CREATE TABLE should_not_exist (id int)", ALL)
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Permission, "{error:?}");
}

#[tokio::test]
async fn server_notices_come_back_with_the_result() {
    let s = connect().await;
    let r = s
        .execute("DO $$ BEGIN RAISE NOTICE 'from the server'; END $$", ALL)
        .await
        .unwrap();
    assert_eq!(r.notices, ["NOTICE: from the server"]);
    assert!(s.execute("SELECT 1", ALL).await.unwrap().notices.is_empty());
}

#[tokio::test]
async fn reconnects_after_the_server_drops_the_connection() {
    let s = connect().await;
    let peer = connect().await;
    let pid = scalar(&s.execute("SELECT pg_backend_pid()", ALL).await.unwrap());
    peer.execute(&format!("SELECT pg_terminate_backend({pid})"), ALL)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // The first statement after the drop may report the loss or already run on
    // a new connection; the one after that must work.
    let _ = s.execute("SELECT 1", ALL).await;
    let r = s.execute("SELECT pg_backend_pid()", ALL).await.unwrap();
    assert_ne!(scalar(&r), pid);
}

#[tokio::test]
async fn pgpass_supplies_the_password_when_none_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pgpass");
    std::fs::write(
        &file,
        format!("{}:{}:rowboat_test:rowboat:rowboat\n", host(), port()),
    )
    .unwrap();
    std::env::set_var("PGPASSFILE", &file);

    let mut p = params();
    p.password = None;
    p.config.use_pgpass = true;
    let s = PgSession::connect(p.clone())
        .await
        .expect("password from pgpass");
    assert_eq!(
        scalar(&s.execute("SELECT current_user", ALL).await.unwrap()),
        "rowboat"
    );

    p.config.use_pgpass = false;
    assert!(
        PgSession::connect(p).await.is_err(),
        "no password without pgpass"
    );
    std::env::remove_var("PGPASSFILE");
}

#[tokio::test]
async fn navigator_lists_objects_and_prints_ddl() {
    let s = connect().await;
    let schema = format!("nav_{}", std::process::id());
    for sql in [
        format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
        format!("CREATE SCHEMA {schema}"),
        format!("CREATE TYPE {schema}.status AS ENUM ('new', 'done')"),
        format!(
            "CREATE TABLE {schema}.customers (id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, \
             email text NOT NULL UNIQUE, created_at timestamptz DEFAULT now())"
        ),
        format!(
            "CREATE TABLE {schema}.orders (id int PRIMARY KEY, customer_id bigint NOT NULL \
             REFERENCES {schema}.customers (id), total numeric(10,2), state {schema}.status)"
        ),
        format!("CREATE INDEX orders_customer ON {schema}.orders (customer_id)"),
        format!(
            "CREATE VIEW {schema}.big_orders AS SELECT id FROM {schema}.orders WHERE total > 100"
        ),
        format!("CREATE SEQUENCE {schema}.ticket START 10"),
        format!(
            "CREATE FUNCTION {schema}.double(n int) RETURNS int LANGUAGE sql AS 'SELECT n * 2'"
        ),
    ] {
        s.execute(&sql, ALL)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
    }
    let names = |nodes: Vec<rowboat_lib::db::SchemaNode>| -> Vec<String> {
        nodes.into_iter().map(|n| n.name).collect()
    };
    let path = |parts: &[&str]| -> Vec<String> { parts.iter().map(|p| p.to_string()).collect() };

    let roots = names(s.children(&[]).await.unwrap());
    assert!(roots.contains(&schema) && roots.contains(&"public".to_string()));
    assert_eq!(
        roots.last().map(String::as_str),
        Some("pg_catalog"),
        "system schemas go last"
    );

    let groups = names(s.children(&path(&[&schema])).await.unwrap());
    assert_eq!(
        groups,
        [
            "Tables",
            "Views",
            "Materialized views",
            "Functions",
            "Sequences",
            "Types"
        ]
    );

    assert_eq!(
        names(s.children(&path(&[&schema, "Tables"])).await.unwrap()),
        ["customers", "orders"]
    );
    assert_eq!(
        names(s.children(&path(&[&schema, "Views"])).await.unwrap()),
        ["big_orders"]
    );
    // The identity column's own sequence is listed too.
    assert!(
        names(s.children(&path(&[&schema, "Sequences"])).await.unwrap())
            .contains(&"ticket".to_string())
    );
    assert_eq!(
        names(s.children(&path(&[&schema, "Types"])).await.unwrap()),
        ["status"]
    );
    let functions = s.children(&path(&[&schema, "Functions"])).await.unwrap();
    assert_eq!(functions[0].name, "double(integer)");
    assert_eq!(functions[0].detail.as_deref(), Some("integer"));

    let columns = s
        .children(&path(&[&schema, "Tables", "orders"]))
        .await
        .unwrap();
    let described: Vec<_> = columns
        .iter()
        .map(|c| format!("{} {}", c.name, c.detail.clone().unwrap_or_default()))
        .collect();
    assert_eq!(
        described,
        [
            "id integer · primary key",
            "customer_id bigint · not null",
            "total numeric(10,2)",
            &format!("state {schema}.status"),
        ]
    );

    let table_ddl = s.ddl(&path(&[&schema, "Tables", "orders"])).await.unwrap();
    assert!(
        table_ddl.starts_with(&format!("CREATE TABLE \"{schema}\".\"orders\" (")),
        "{table_ddl}"
    );
    assert!(
        table_ddl.contains("customer_id bigint NOT NULL"),
        "{table_ddl}"
    );
    assert!(table_ddl.contains("PRIMARY KEY (id)"), "{table_ddl}");
    assert!(
        table_ddl.contains(&format!("REFERENCES {schema}.customers(id)")),
        "{table_ddl}"
    );
    assert!(
        table_ddl.contains("CREATE INDEX orders_customer"),
        "{table_ddl}"
    );

    // The generated DDL must be valid SQL: recreate the table from it.
    s.execute(&format!("DROP VIEW {schema}.big_orders"), ALL)
        .await
        .unwrap();
    s.execute(&format!("DROP TABLE {schema}.orders"), ALL)
        .await
        .unwrap();
    for statement in table_ddl.split(";\n\n") {
        s.execute(statement.trim_end_matches(';'), ALL)
            .await
            .unwrap_or_else(|e| panic!("generated DDL failed: {statement}: {e:?}"));
    }

    let identity = s
        .ddl(&path(&[&schema, "Tables", "customers"]))
        .await
        .unwrap();
    assert!(
        identity.contains("id bigint GENERATED ALWAYS AS IDENTITY NOT NULL"),
        "{identity}"
    );
    assert!(
        identity.contains("created_at timestamp with time zone DEFAULT now()"),
        "{identity}"
    );

    let function = s
        .ddl(&path(&[&schema, "Functions", "double(integer)"]))
        .await
        .unwrap();
    assert!(
        function.starts_with("CREATE OR REPLACE FUNCTION"),
        "{function}"
    );
    let sequence = s
        .ddl(&path(&[&schema, "Sequences", "ticket"]))
        .await
        .unwrap();
    assert!(sequence.contains("START WITH 10"), "{sequence}");
    let enum_type = s.ddl(&path(&[&schema, "Types", "status"])).await.unwrap();
    assert_eq!(
        enum_type,
        format!("CREATE TYPE \"{schema}\".\"status\" AS ENUM ('new', 'done');")
    );

    s.execute(&format!("DROP SCHEMA {schema} CASCADE"), ALL)
        .await
        .unwrap();
}

// ------------------------------------------------------------------ TLS ----

fn tls_dir() -> Option<std::path::PathBuf> {
    match std::env::var_os("ROWBOAT_TEST_PG_TLS_DIR") {
        Some(dir) => Some(dir.into()),
        None => {
            eprintln!("skipped: set ROWBOAT_TEST_PG_TLS_DIR to run the TLS tests");
            None
        }
    }
}

async fn is_encrypted(s: &PgSession) -> String {
    scalar(
        &s.execute(
            "SELECT ssl::text FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            ALL,
        )
        .await
        .unwrap(),
    )
}

#[tokio::test]
async fn tls_modes_encrypt_and_verify_as_documented() {
    let Some(dir) = tls_dir() else { return };
    let ca = dir.join("ca.crt").display().to_string();

    let mut plain = params();
    plain.config.tls.mode = TlsMode::Disable;
    assert_eq!(
        is_encrypted(&PgSession::connect(plain).await.unwrap()).await,
        "false"
    );

    for mode in [TlsMode::Prefer, TlsMode::Require] {
        let mut p = params();
        p.config.tls.mode = mode;
        assert_eq!(
            is_encrypted(&PgSession::connect(p).await.unwrap()).await,
            "true",
            "{mode:?}"
        );
    }

    // verify-full: the right CA and the right name.
    let mut full = params();
    full.config.host = "localhost".into();
    full.config.tls.mode = TlsMode::VerifyFull;
    full.config.tls.ca_cert_path = ca.clone();
    assert_eq!(
        is_encrypted(&PgSession::connect(full.clone()).await.unwrap()).await,
        "true"
    );

    // verify-full without the CA: the system store does not know this server.
    let mut untrusted = full.clone();
    untrusted.config.tls.ca_cert_path.clear();
    let error = PgSession::connect(untrusted).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Connection, "{error:?}");

    // A name the certificate does not cover, reached through a forwarded
    // address (as an SSH tunnel does): verify-full refuses, verify-ca accepts.
    let mut other_name = full.clone();
    other_name.config.host = "db.internal.example".into();
    other_name.via = Some(format!("127.0.0.1:{}", port()).parse().unwrap());
    let error = PgSession::connect(other_name.clone()).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Connection, "{error:?}");

    other_name.config.tls.mode = TlsMode::VerifyCa;
    assert_eq!(
        is_encrypted(&PgSession::connect(other_name).await.unwrap()).await,
        "true"
    );
}

#[tokio::test]
async fn client_certificate_authenticates_without_a_password() {
    let Some(dir) = tls_dir() else { return };
    let mut p = params();
    p.password = None;
    p.config.host = "localhost".into();
    p.config.user = "certuser".into();
    p.config.tls.mode = TlsMode::VerifyFull;
    p.config.tls.ca_cert_path = dir.join("ca.crt").display().to_string();
    p.config.tls.client_cert_path = dir.join("client.crt").display().to_string();
    p.config.tls.client_key_path = dir.join("client.key").display().to_string();
    let s = PgSession::connect(p.clone())
        .await
        .expect("certificate login");
    assert_eq!(
        scalar(&s.execute("SELECT current_user", ALL).await.unwrap()),
        "certuser"
    );

    p.config.tls.client_cert_path.clear();
    p.config.tls.client_key_path.clear();
    assert!(
        PgSession::connect(p).await.is_err(),
        "no certificate, no login"
    );
}
