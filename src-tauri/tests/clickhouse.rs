//! ClickHouse driver tests against a real server.
//!
//!     docker compose up -d --wait
//!     cargo test --features integration --test clickhouse
//!
//! Server location comes from `ROWBOAT_TEST_CH_HOST` / `_PORT` (default
//! 127.0.0.1:8123, the docker-compose service).
//!
//! The unit tests in `src/db/clickhouse.rs` cover the same behaviour against a
//! fake server; these prove it against the real protocol.
#![cfg(feature = "integration")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use rowboat_lib::connections::{ConnectionConfig, Tls, TlsMode};
use rowboat_lib::db::clickhouse::ChSession;
use rowboat_lib::db::{ConnectParams, DbKind, ErrorKind, ExecOptions, QueryResult, Session};

const ALL: ExecOptions = ExecOptions::limit(0);

fn params() -> ConnectParams {
    ConnectParams {
        config: ConnectionConfig {
            name: "test".into(),
            kind: DbKind::Clickhouse,
            host: std::env::var("ROWBOAT_TEST_CH_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port: std::env::var("ROWBOAT_TEST_CH_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8123),
            database: "rowboat_test".into(),
            user: "rowboat".into(),
            tls: Tls {
                mode: TlsMode::Disable,
                ..Default::default()
            },
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

async fn connect() -> ChSession {
    ChSession::connect(params())
        .await
        .expect("connect to the test server")
}

fn scalar(result: &QueryResult) -> String {
    result.rows[0][0].clone().unwrap_or_else(|| "NULL".into())
}

#[tokio::test]
async fn connects_and_reports_the_server_version() {
    let s = connect().await;
    assert!(s.server_version().starts_with("ClickHouse "));
    assert_eq!(scalar(&s.execute("SELECT 1", ALL).await.unwrap()), "1");
}

#[tokio::test]
async fn client_id_is_visible_to_the_server() {
    let mut p = params();
    p.client_id = "rowboat-dominic".into();
    let s = ChSession::connect(p).await.unwrap();
    // The running query can see its own client name in system.processes.
    let r = s
        .execute(
            "SELECT http_user_agent FROM system.processes WHERE query_id = queryID()",
            ALL,
        )
        .await
        .unwrap();
    assert_eq!(scalar(&r), "rowboat-dominic");
    let comment = s
        .execute("SELECT getSetting('log_comment')", ALL)
        .await
        .unwrap();
    assert_eq!(scalar(&comment), "rowboat-dominic");
}

#[tokio::test]
async fn row_limit_fetches_one_page_of_an_endless_table() {
    let s = connect().await;
    let started = Instant::now();
    // system.numbers never ends.
    let r = s
        .execute(
            "SELECT number FROM system.numbers",
            ExecOptions::limit(1000),
        )
        .await
        .unwrap();
    assert_eq!(r.rows.len(), 1000);
    assert!(r.more_available);
    assert_eq!(r.rows[999][0].as_deref(), Some("999"));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn exactly_the_limit_is_not_reported_as_more() {
    let s = connect().await;
    let r = s
        .execute("SELECT number FROM numbers(50)", ExecOptions::limit(50))
        .await
        .unwrap();
    assert_eq!(r.rows.len(), 50);
    assert!(!r.more_available);
}

/// Each value must render as ClickHouse's own `toString` does.
#[tokio::test]
async fn values_render_like_the_servers_text_output() {
    let s = connect().await;
    let expressions = [
        "toUInt8(255)",
        "toInt8(-1)",
        "toUInt64(18446744073709551615)",
        "toInt64(-9223372036854775808)",
        "toUInt128('340282366920938463463374607431768211455')",
        "toInt256('-12345678901234567890123456789012345678901234567890')",
        "toFloat64(1.5)",
        "toFloat32(0.1)",
        "toDecimal64(123.45, 2)",
        "toDecimal128('-0.0005', 10)",
        "toDecimal32(1.5, 3)",
        "'héllo'",
        "toFixedString('ab', 5)",
        "toLowCardinality('lc')",
        "toDate('2024-02-29')",
        "toDate32('1969-12-31')",
        "toDateTime('2024-02-29 13:14:15', 'UTC')",
        "toDateTime64('2024-02-29 13:14:15.123', 3, 'UTC')",
        "toUUID('61f0c404-5cb3-11e7-907b-a6006ad3dba0')",
        "toIPv4('192.168.1.10')",
        "toIPv6('2001:db8::1')",
        "true",
        "CAST('active', 'Enum8(\\'active\\' = 1, \\'gone\\' = 2)')",
        "[1, 2, 3]",
        "['a', 'it\\'s']",
        "[[1], []]",
        "[toNullable(5), NULL]",
        "(1, 'x')",
        "map('k', 2)",
        "tuple(toDate('2024-01-01'), [1.5])",
    ];
    for expression in expressions {
        let r = s
            .execute(&format!("SELECT ({expression}) AS v, toString(v)"), ALL)
            .await
            .unwrap_or_else(|e| panic!("{expression}: {e:?}"));
        assert_eq!(r.rows[0][0], r.rows[0][1], "{expression}");
    }
    let null = s.execute("SELECT NULL, toNullable(1)", ALL).await.unwrap();
    assert_eq!(null.rows[0], vec![None, Some("1".into())]);
}

#[tokio::test]
async fn errors_are_classified() {
    let s = connect().await;
    let syntax = s
        .execute("SELECT 1 FROMM system.one", ALL)
        .await
        .unwrap_err();
    assert_eq!(syntax.kind, ErrorKind::Syntax, "{syntax:?}");
    assert!(syntax.position.is_some());

    let schema = s
        .execute("SELECT * FROM no_such_table", ALL)
        .await
        .unwrap_err();
    assert_eq!(schema.kind, ErrorKind::Schema, "{schema:?}");

    assert_eq!(scalar(&s.execute("SELECT 1", ALL).await.unwrap()), "1");
}

#[tokio::test]
async fn wrong_password_is_a_permission_error() {
    let mut p = params();
    p.password = Some("nope".into());
    let error = ChSession::connect(p).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Permission, "{error:?}");
}

#[tokio::test]
async fn cancel_stops_a_long_query_within_a_second() {
    let s = Arc::new(connect().await);
    let runner = s.clone();
    let task = tokio::spawn(async move {
        runner
            .execute(
                "SELECT count() FROM system.numbers WHERE NOT ignore(sleepEachRow(0.001))",
                ALL,
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    let cancelled_at = Instant::now();
    s.cancel().await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(1),
        "{:?}",
        cancelled_at.elapsed()
    );

    // The server really stopped it: nothing of ours is still running.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let still_running = s
        .execute(
            "SELECT count() FROM system.processes WHERE query LIKE '%sleepEachRow%' AND query_id != queryID()",
            ALL,
        )
        .await
        .unwrap();
    assert_eq!(scalar(&still_running), "0");
}

#[tokio::test]
async fn read_only_connections_refuse_writes_but_still_limit_rows() {
    let mut p = params();
    p.config.read_only = true;
    let s = ChSession::connect(p).await.unwrap();
    let r = s
        .execute("SELECT number FROM system.numbers", ExecOptions::limit(10))
        .await
        .unwrap();
    assert_eq!(r.rows.len(), 10);
    let error = s
        .execute(
            "CREATE TABLE should_not_exist (n UInt8) ENGINE = Memory",
            ALL,
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Permission, "{error:?}");
}

#[tokio::test]
async fn session_state_and_format_clause() {
    let s = connect().await;
    // SET persists within the connection's session.
    s.execute("SET max_threads = 3", ALL).await.unwrap();
    assert_eq!(
        scalar(
            &s.execute("SELECT getSetting('max_threads')", ALL)
                .await
                .unwrap()
        ),
        "3"
    );

    let raw = s
        .execute("SELECT 1 AS n FORMAT JSONEachRow", ALL)
        .await
        .unwrap();
    assert_eq!(raw.raw.as_deref().map(str::trim), Some("{\"n\":1}"));
}

#[tokio::test]
async fn writes_and_navigator() {
    let s = connect().await;
    let table = format!("it_events_{}", std::process::id());
    s.execute(
        &format!(
            "CREATE TABLE {table} (user_id UInt64, ts DateTime, payload String) \
             ENGINE = MergeTree PARTITION BY toYYYYMM(ts) ORDER BY (user_id, ts)"
        ),
        ALL,
    )
    .await
    .unwrap();
    let insert = s
        .execute(
            &format!("INSERT INTO {table} SELECT number, now(), 'x' FROM numbers(42)"),
            ALL,
        )
        .await
        .unwrap();
    assert_eq!(insert.rows_affected, Some(42));

    let path = |parts: &[&str]| -> Vec<String> { parts.iter().map(|p| p.to_string()).collect() };
    let databases: Vec<_> = s
        .children(&[])
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert!(databases.contains(&"rowboat_test".to_string()));

    let tables = s
        .children(&path(&["rowboat_test", "Tables"]))
        .await
        .unwrap();
    let ours = tables
        .iter()
        .find(|t| t.name == table)
        .expect("table listed");
    assert_eq!(
        ours.detail.as_deref(),
        Some("MergeTree · ORDER BY user_id, ts · PARTITION BY toYYYYMM(ts)")
    );
    let columns = s
        .children(&path(&["rowboat_test", "Tables", &table]))
        .await
        .unwrap();
    assert_eq!(columns[0].detail.as_deref(), Some("UInt64 · sorting key"));
    assert_eq!(columns[2].detail.as_deref(), Some("String"));

    let ddl = s
        .ddl(&path(&["rowboat_test", "Tables", &table]))
        .await
        .unwrap();
    assert!(ddl.starts_with("CREATE TABLE rowboat_test."), "{ddl}");
    assert!(ddl.contains("ORDER BY (user_id, ts)"), "{ddl}");

    s.execute(&format!("DROP TABLE {table}"), ALL)
        .await
        .unwrap();
}
