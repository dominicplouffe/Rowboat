//! SSH tunnel tests against a real sshd, forwarding to the PostgreSQL test
//! server.
//!
//!     eval "$(scripts/dev-sshd.sh)"     # starts sshd and exports the variables
//!     cargo test --features integration --test ssh
//!
//! Needs `ROWBOAT_TEST_SSH_PORT`, `ROWBOAT_TEST_SSH_USER` and
//! `ROWBOAT_TEST_SSH_KEY` (a private key that user accepts). Optional:
//! `ROWBOAT_TEST_SSH_PASSWORD`, and `ROWBOAT_TEST_SSH_ENCRYPTED_KEY` with
//! `ROWBOAT_TEST_SSH_PASSPHRASE`. Without the required ones every test is
//! skipped with a message.
#![cfg(feature = "integration")]

use std::path::Path;
use std::time::Duration;

use rowboat_lib::connections::{ConnectionConfig, Ssh, SshAuth};
use rowboat_lib::db::postgres::PgSession;
use rowboat_lib::db::{ConnectParams, DbKind, ErrorKind, ExecOptions, Session};
use rowboat_lib::settings::KnownHostsPolicy;
use rowboat_lib::ssh::{Tunnel, TunnelParams};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

struct Target {
    port: u16,
    user: String,
    key: String,
}

fn target() -> Option<Target> {
    let found = (|| {
        Some(Target {
            port: env("ROWBOAT_TEST_SSH_PORT")?.parse().ok()?,
            user: env("ROWBOAT_TEST_SSH_USER")?,
            key: env("ROWBOAT_TEST_SSH_KEY")?,
        })
    })();
    if found.is_none() {
        eprintln!("skipped: set ROWBOAT_TEST_SSH_PORT, _USER and _KEY to run the SSH tunnel tests");
    }
    found
}

fn pg_port() -> u16 {
    env("ROWBOAT_TEST_PG_PORT")
        .and_then(|p| p.parse().ok())
        .unwrap_or(5432)
}

fn params(target: &Target, known_hosts: &Path) -> TunnelParams {
    TunnelParams {
        ssh: Ssh {
            enabled: true,
            host: "127.0.0.1".into(),
            port: target.port,
            user: target.user.clone(),
            auth: SshAuth::Key,
            key_path: target.key.clone(),
        },
        password: None,
        passphrase: None,
        // As seen from the SSH server.
        target_host: "127.0.0.1".into(),
        target_port: pg_port(),
        policy: KnownHostsPolicy::AcceptNew,
        trusted_fingerprint: None,
        connect_timeout: Duration::from_secs(10),
        keepalive: Duration::from_secs(30),
        known_hosts_path: known_hosts.to_path_buf(),
    }
}

/// Opens a PostgreSQL session through the tunnel and proves it works.
async fn query_through(tunnel: &Tunnel) -> String {
    let session = PgSession::connect(ConnectParams {
        config: ConnectionConfig {
            name: "via ssh".into(),
            kind: DbKind::Postgres,
            // A name that only resolves through the tunnel.
            host: "db.internal.example".into(),
            port: 5432,
            database: "rowboat_test".into(),
            user: "rowboat".into(),
            use_pgpass: false,
            ..Default::default()
        },
        password: Some("rowboat".into()),
        client_id: "rowboat-ssh-test".into(),
        connect_timeout: Duration::from_secs(10),
        keepalive: Duration::ZERO,
        auto_commit: true,
        auto_reconnect: false,
        via: Some(tunnel.local),
    })
    .await
    .expect("connect through the tunnel");
    let result = session
        .execute(
            "SELECT count(*) FROM generate_series(1, 100000)",
            ExecOptions { row_limit: 0 },
        )
        .await
        .unwrap();
    result.rows[0][0].clone().unwrap()
}

#[tokio::test]
async fn key_authentication_forwards_queries() {
    let Some(target) = target() else { return };
    let dir = tempfile::tempdir().unwrap();
    let tunnel = Tunnel::open(params(&target, &dir.path().join("known_hosts")))
        .await
        .unwrap();
    assert!(tunnel.local.ip().is_loopback());
    assert_eq!(query_through(&tunnel).await, "100000");
}

#[tokio::test]
async fn several_connections_share_one_tunnel() {
    let Some(target) = target() else { return };
    let dir = tempfile::tempdir().unwrap();
    let tunnel = Tunnel::open(params(&target, &dir.path().join("known_hosts")))
        .await
        .unwrap();
    let (a, b, c) = tokio::join!(
        query_through(&tunnel),
        query_through(&tunnel),
        query_through(&tunnel)
    );
    assert_eq!(
        (a.as_str(), b.as_str(), c.as_str()),
        ("100000", "100000", "100000")
    );
}

#[tokio::test]
async fn closing_the_tunnel_closes_the_local_port() {
    let Some(target) = target() else { return };
    let dir = tempfile::tempdir().unwrap();
    let tunnel = Tunnel::open(params(&target, &dir.path().join("known_hosts")))
        .await
        .unwrap();
    let local = tunnel.local;
    drop(tunnel);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(tokio::net::TcpStream::connect(local).await.is_err());
}

#[tokio::test]
async fn password_authentication() {
    let Some(target) = target() else { return };
    let Some(password) = env("ROWBOAT_TEST_SSH_PASSWORD") else {
        eprintln!("skipped: set ROWBOAT_TEST_SSH_PASSWORD");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut p = params(&target, &dir.path().join("known_hosts"));
    p.ssh.auth = SshAuth::Password;
    p.password = Some(password);
    let tunnel = Tunnel::open(p).await.unwrap();
    assert_eq!(query_through(&tunnel).await, "100000");

    let mut wrong = params(&target, &dir.path().join("known_hosts"));
    wrong.ssh.auth = SshAuth::Password;
    wrong.password = Some("definitely not it".into());
    let error = Tunnel::open(wrong).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Permission, "{error:?}");
}

#[tokio::test]
async fn encrypted_key_needs_its_passphrase() {
    let Some(target) = target() else { return };
    let (Some(key), Some(passphrase)) = (
        env("ROWBOAT_TEST_SSH_ENCRYPTED_KEY"),
        env("ROWBOAT_TEST_SSH_PASSPHRASE"),
    ) else {
        eprintln!("skipped: set ROWBOAT_TEST_SSH_ENCRYPTED_KEY and _PASSPHRASE");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut p = params(&target, &dir.path().join("known_hosts"));
    p.ssh.key_path = key.clone();

    // Without the passphrase the key cannot even be loaded.
    let error = Tunnel::open(TunnelParams {
        ssh: p.ssh.clone(),
        ..params(&target, &dir.path().join("known_hosts"))
    })
    .await
    .err()
    .unwrap();
    assert_eq!(error.kind, ErrorKind::Permission, "{error:?}");
    assert!(error.hint.unwrap().contains("passphrase"));

    p.passphrase = Some(passphrase);
    let tunnel = Tunnel::open(p).await.unwrap();
    assert_eq!(query_through(&tunnel).await, "100000");
}

#[tokio::test]
async fn ssh_agent_authentication() {
    let Some(target) = target() else { return };
    if env("SSH_AUTH_SOCK").is_none() {
        eprintln!("skipped: no SSH agent (SSH_AUTH_SOCK)");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut p = params(&target, &dir.path().join("known_hosts"));
    p.ssh.auth = SshAuth::Agent;
    p.ssh.key_path.clear();
    let tunnel = Tunnel::open(p).await.unwrap();
    assert_eq!(query_through(&tunnel).await, "100000");
}

#[tokio::test]
async fn unknown_host_keys_follow_the_policy() {
    let Some(target) = target() else { return };
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = dir.path().join("known_hosts");

    // Strict: refuse, and record nothing.
    let mut strict = params(&target, &known_hosts);
    strict.policy = KnownHostsPolicy::Strict;
    let error = Tunnel::open(strict).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::UnknownHostKey, "{error:?}");
    assert!(!known_hosts.exists());

    // Ask: refuse first, and hand the fingerprint to the UI…
    let mut ask = params(&target, &known_hosts);
    ask.policy = KnownHostsPolicy::Ask;
    let error = Tunnel::open(ask).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::UnknownHostKey);
    let fingerprint = error.detail.expect("fingerprint for the prompt");
    assert!(fingerprint.starts_with("SHA256:"), "{fingerprint}");

    // …a different fingerprint than the one the user approved is still refused…
    let mut wrong = params(&target, &known_hosts);
    wrong.policy = KnownHostsPolicy::Ask;
    wrong.trusted_fingerprint = Some("SHA256:not-the-one-shown".into());
    assert!(Tunnel::open(wrong).await.is_err());

    // …and the approved one is accepted and remembered.
    let mut trusted = params(&target, &known_hosts);
    trusted.policy = KnownHostsPolicy::Ask;
    trusted.trusted_fingerprint = Some(fingerprint);
    Tunnel::open(trusted).await.unwrap();
    assert!(std::fs::read_to_string(&known_hosts)
        .unwrap()
        .contains("ssh-"));

    // Now even Strict connects, because the key is known.
    let mut strict = params(&target, &known_hosts);
    strict.policy = KnownHostsPolicy::Strict;
    Tunnel::open(strict).await.unwrap();
}

#[tokio::test]
async fn a_changed_host_key_is_always_refused() {
    let Some(target) = target() else { return };
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = dir.path().join("known_hosts");
    // Record a different key for this host, as if the server had been replaced.
    std::fs::write(
        &known_hosts,
        format!(
            "[127.0.0.1]:{} ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ\n",
            target.port
        ),
    )
    .unwrap();
    for policy in [
        KnownHostsPolicy::AcceptNew,
        KnownHostsPolicy::Ask,
        KnownHostsPolicy::Strict,
    ] {
        let mut p = params(&target, &known_hosts);
        p.policy = policy;
        let error = Tunnel::open(p).await.err().unwrap();
        assert_eq!(error.kind, ErrorKind::Connection, "{policy:?}: {error:?}");
        assert!(error.message.contains("CHANGED"), "{error:?}");
    }
}

#[tokio::test]
async fn an_unreachable_target_fails_when_the_tunnel_opens() {
    let Some(target) = target() else { return };
    let dir = tempfile::tempdir().unwrap();
    let mut p = params(&target, &dir.path().join("known_hosts"));
    p.target_port = 1; // nothing listens there
    let error = Tunnel::open(p).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Connection, "{error:?}");
}
