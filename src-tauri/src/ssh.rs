//! SSH tunnels (`russh`, pure Rust).
//!
//! A tunnel is a local listener on 127.0.0.1 whose connections are forwarded
//! through an SSH session to the database host. The drivers connect to the
//! local address while still verifying TLS against the real host name.
//!
//! Host keys are checked against `~/.ssh/known_hosts`, the same file OpenSSH
//! uses. An unknown key is handled per the user's policy; a *changed* key is
//! always refused.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::agent::client::AgentClient;
use russh::keys::{known_hosts, HashAlg, PrivateKeyWithHashAlg, PublicKey};
use tokio::net::TcpListener;

use crate::connections::{Ssh, SshAuth};
use crate::db::{DbError, DbResult, ErrorKind};
use crate::settings::KnownHostsPolicy;

pub struct TunnelParams {
    pub ssh: Ssh,
    pub password: Option<String>,
    pub passphrase: Option<String>,
    /// Where the tunnel leads: the database host as seen from the SSH server.
    pub target_host: String,
    pub target_port: u16,
    pub policy: KnownHostsPolicy,
    /// A fingerprint the user has just agreed to trust (policy `Ask`).
    pub trusted_fingerprint: Option<String>,
    pub connect_timeout: Duration,
    /// Zero disables keepalive.
    pub keepalive: Duration,
    pub known_hosts_path: PathBuf,
}

/// The user's `~/.ssh/known_hosts`.
pub fn default_known_hosts_path() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".ssh").join("known_hosts")
}

#[derive(Debug, Clone)]
enum HostKeyIssue {
    Unknown {
        fingerprint: String,
        algorithm: String,
    },
    Changed {
        line: usize,
    },
    /// known_hosts exists but could not be read or understood, so whether
    /// this host is known cannot be decided.
    Unreadable {
        reason: String,
    },
}

struct Checker {
    host: String,
    port: u16,
    policy: KnownHostsPolicy,
    trusted_fingerprint: Option<String>,
    known_hosts_path: PathBuf,
    issue: Arc<Mutex<Option<HostKeyIssue>>>,
}

impl Checker {
    fn decide(&self, key: &PublicKey) -> bool {
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let report = |issue| *self.issue.lock().unwrap_or_else(|e| e.into_inner()) = Some(issue);
        // A missing file simply means no host is known yet.
        let listed = if self.known_hosts_path.exists() {
            known_hosts::check_known_hosts_path(&self.host, self.port, key, &self.known_hosts_path)
        } else {
            Ok(false)
        };
        match listed {
            Ok(true) => true,
            Err(russh::keys::Error::KeyChanged { line }) => {
                report(HostKeyIssue::Changed { line });
                false
            }
            // The file is there but unusable. Treating that as "unknown host"
            // would let a changed key through under the accept-new policy.
            Err(error) => {
                report(HostKeyIssue::Unreadable {
                    reason: error.to_string(),
                });
                false
            }
            // Not listed (or no known_hosts file yet).
            Ok(false) => {
                let accept = match self.policy {
                    KnownHostsPolicy::AcceptNew => true,
                    KnownHostsPolicy::Ask => {
                        self.trusted_fingerprint.as_deref() == Some(&fingerprint)
                    }
                    KnownHostsPolicy::Strict => false,
                };
                if accept {
                    if let Some(dir) = self.known_hosts_path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    // Failing to record the key is not a reason to refuse it.
                    let _ = known_hosts::learn_known_hosts_path(
                        &self.host,
                        self.port,
                        key,
                        &self.known_hosts_path,
                    );
                } else {
                    report(HostKeyIssue::Unknown {
                        fingerprint,
                        algorithm: key.algorithm().to_string(),
                    });
                }
                accept
            }
        }
    }
}

impl client::Handler for Checker {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(match server_public_key {
            russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => self.decide(key),
            // Host certificates need a trusted CA list, which is not supported yet.
            russh::keys::PublicKeyOrCertificate::Certificate(_) => false,
        })
    }
}

/// An open tunnel. Dropping it closes the listener and the SSH session.
pub struct Tunnel {
    pub local: SocketAddr,
    forwarder: tokio::task::JoinHandle<()>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}

fn failed(what: &str, error: impl std::fmt::Display) -> DbError {
    DbError::connection(format!("SSH tunnel: {what}: {error}"))
}

async fn authenticate(session: &mut Handle<Checker>, params: &TunnelParams) -> DbResult<()> {
    let user = params.ssh.user.clone();
    let rejected = |how: &str| {
        DbError::new(
            ErrorKind::Permission,
            format!("SSH tunnel: the server rejected {how} for user {user}."),
        )
    };
    match params.ssh.auth {
        SshAuth::Password => {
            let password = params.password.clone().unwrap_or_default();
            let result = session
                .authenticate_password(user.clone(), password)
                .await
                .map_err(|e| failed("password authentication failed", e))?;
            if !result.success() {
                return Err(rejected("the password"));
            }
        }
        SshAuth::Key => {
            let passphrase = params.passphrase.as_deref().filter(|p| !p.is_empty());
            let key =
                russh::keys::load_secret_key(&params.ssh.key_path, passphrase).map_err(|e| {
                    let text = e.to_string();
                    let encrypted = text.to_lowercase().contains("encrypt")
                        || text.to_lowercase().contains("passphrase")
                        || text.to_lowercase().contains("decrypt");
                    let mut error = DbError::new(
                        ErrorKind::Permission,
                        format!(
                            "SSH tunnel: could not load the private key {}: {text}",
                            params.ssh.key_path
                        ),
                    );
                    if encrypted {
                        error.hint = Some(if passphrase.is_some() {
                            "The passphrase may be wrong.".into()
                        } else {
                            "This key is encrypted: enter its passphrase.".into()
                        });
                    }
                    error
                })?;
            let hash = session
                .best_supported_rsa_hash()
                .await
                .map_err(|e| failed("negotiation failed", e))?
                .flatten();
            let result = session
                .authenticate_publickey(
                    user.clone(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await
                .map_err(|e| failed("key authentication failed", e))?;
            if !result.success() {
                return Err(rejected("the private key"));
            }
        }
        SshAuth::Agent => {
            #[cfg(unix)]
            let agent = AgentClient::connect_env().await.map(AgentClient::dynamic);
            #[cfg(windows)]
            let agent = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")
                .await
                .map(AgentClient::dynamic);
            let mut agent = agent.map_err(|e| {
                DbError::new(
                    ErrorKind::Permission,
                    format!("SSH tunnel: could not reach the SSH agent: {e}"),
                )
                .with_hint("Start ssh-agent and add your key with ssh-add, or switch to key or password authentication.")
            })?;
            let identities = agent
                .request_identities()
                .await
                .map_err(|e| failed("the SSH agent did not list its keys", e))?;
            if identities.is_empty() {
                return Err(DbError::new(
                    ErrorKind::Permission,
                    "SSH tunnel: the SSH agent holds no keys.",
                )
                .with_hint("Add a key with ssh-add."));
            }
            let hash = session
                .best_supported_rsa_hash()
                .await
                .map_err(|e| failed("negotiation failed", e))?
                .flatten();
            let mut accepted = false;
            for identity in identities {
                let key = identity.public_key().into_owned();
                let result = session
                    .authenticate_publickey_with(user.clone(), key, hash, &mut agent)
                    .await;
                if matches!(result, Ok(r) if r.success()) {
                    accepted = true;
                    break;
                }
            }
            if !accepted {
                return Err(rejected("every key in the SSH agent"));
            }
        }
    }
    Ok(())
}

impl Tunnel {
    pub async fn open(params: TunnelParams) -> DbResult<Self> {
        let config = Arc::new(client::Config {
            keepalive_interval: (!params.keepalive.is_zero()).then_some(params.keepalive),
            ..Default::default()
        });
        let issue = Arc::new(Mutex::new(None));
        let checker = Checker {
            host: params.ssh.host.clone(),
            port: params.ssh.port,
            policy: params.policy,
            trusted_fingerprint: params.trusted_fingerprint.clone(),
            known_hosts_path: params.known_hosts_path.clone(),
            issue: issue.clone(),
        };

        let address = (params.ssh.host.clone(), params.ssh.port);
        let connecting = client::connect(config, address, checker);
        let mut session = match tokio::time::timeout(params.connect_timeout, connecting).await {
            Err(_) => {
                return Err(DbError::new(
                    ErrorKind::Timeout,
                    format!(
                        "SSH tunnel: timed out connecting to {}:{}.",
                        params.ssh.host, params.ssh.port
                    ),
                ))
            }
            Ok(Err(error)) => {
                let issue = issue.lock().unwrap_or_else(|e| e.into_inner()).take();
                return Err(match issue {
                    Some(HostKeyIssue::Unknown { fingerprint, algorithm }) => DbError {
                        kind: ErrorKind::UnknownHostKey,
                        message: format!(
                            "The SSH server {}:{} is not in your known_hosts file.",
                            params.ssh.host, params.ssh.port
                        ),
                        // The UI shows this and sends it back if the user trusts it.
                        detail: Some(fingerprint),
                        hint: Some(format!("{algorithm} key")),
                        code: None,
                        position: None,
                    },
                    Some(HostKeyIssue::Changed { line }) => DbError::connection(format!(
                        "SSH tunnel: the host key of {}:{} has CHANGED since you last connected. \
                         This can mean someone is intercepting the connection.",
                        params.ssh.host, params.ssh.port
                    ))
                    .with_hint(format!(
                        "If the server was really reinstalled, remove line {line} of {} and connect again.",
                        params.known_hosts_path.display()
                    )),
                    Some(HostKeyIssue::Unreadable { reason }) => DbError::connection(format!(
                        "SSH tunnel: could not check the host key because {} cannot be read: {reason}",
                        params.known_hosts_path.display()
                    ))
                    .with_hint("Fix or remove that file, then connect again."),
                    None => failed(&format!("could not connect to {}:{}", params.ssh.host, params.ssh.port), error),
                });
            }
            Ok(Ok(session)) => session,
        };

        authenticate(&mut session, &params).await?;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| failed("could not open a local port", e))?;
        let local = listener
            .local_addr()
            .map_err(|e| failed("could not open a local port", e))?;

        // Fail now, not at the first query, if the server refuses forwarding.
        let probe = session
            .channel_open_direct_tcpip(
                params.target_host.clone(),
                params.target_port as u32,
                "127.0.0.1",
                local.port() as u32,
            )
            .await
            .map_err(|e| {
                failed(
                    &format!(
                        "the SSH server could not reach {}:{} (or does not allow forwarding)",
                        params.target_host, params.target_port
                    ),
                    e,
                )
            })?;
        drop(probe);

        let session = Arc::new(session);
        let (target_host, target_port) = (params.target_host.clone(), params.target_port as u32);
        let forwarder = tokio::spawn(async move {
            loop {
                let Ok((mut inbound, peer)) = listener.accept().await else {
                    break;
                };
                let session = session.clone();
                let target_host = target_host.clone();
                tokio::spawn(async move {
                    let Ok(channel) = session
                        .channel_open_direct_tcpip(
                            target_host,
                            target_port,
                            peer.ip().to_string(),
                            peer.port() as u32,
                        )
                        .await
                    else {
                        return;
                    };
                    let mut outbound = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                });
            }
        });

        Ok(Self { local, forwarder })
    }
}
