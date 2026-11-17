//! Integration-test harness smoke test.
//!
//! Proves the PostgreSQL and ClickHouse services from `docker-compose.yml`
//! (or CI's service containers) are reachable, so that the driver tests added
//! in M1 start from a known-good harness. Deliberately dependency-free: the
//! real drivers arrive with M1.
//!
//!     docker compose up -d --wait
//!     cargo test --features integration
#![cfg(feature = "integration")]

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn connect(addr: &str) -> TcpStream {
    let socket = addr
        .to_socket_addrs()
        .unwrap_or_else(|e| panic!("cannot resolve {addr}: {e}"))
        .next()
        .unwrap_or_else(|| panic!("no address for {addr}"));
    let stream = TcpStream::connect_timeout(&socket, TIMEOUT).unwrap_or_else(|e| {
        panic!("cannot connect to {addr}: {e}. Is `docker compose up` running?")
    });
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    stream.set_write_timeout(Some(TIMEOUT)).unwrap();
    stream
}

/// PostgreSQL answers an SSLRequest with a single byte: 'S' or 'N'.
#[test]
fn postgres_speaks_the_postgres_protocol() {
    let addr = env_or("ROWBOAT_TEST_POSTGRES", "127.0.0.1:5432");
    let mut stream = connect(&addr);
    // Int32 length (8) + Int32 SSLRequest code (80877103).
    stream
        .write_all(&[0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f])
        .unwrap();
    let mut reply = [0u8; 1];
    stream.read_exact(&mut reply).unwrap();
    assert!(
        matches!(reply[0], b'S' | b'N'),
        "unexpected reply to SSLRequest: {:?}",
        reply[0] as char
    );
}

/// ClickHouse's HTTP interface answers `/ping` with "Ok.".
#[test]
fn clickhouse_http_interface_answers_ping() {
    let addr = env_or("ROWBOAT_TEST_CLICKHOUSE", "127.0.0.1:8123");
    let mut stream = connect(&addr);
    stream
        .write_all(b"GET /ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.trim_end().ends_with("Ok."), "{response}");
}
