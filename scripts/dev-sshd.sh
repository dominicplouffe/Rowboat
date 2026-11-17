#!/usr/bin/env bash
# Starts a throwaway sshd on 127.0.0.1 for the SSH tunnel tests, with a fresh
# host key and a fresh key pair that the current user accepts.
#
#   eval "$(scripts/dev-sshd.sh)"     # start, and export ROWBOAT_TEST_SSH_*
#   cargo test --manifest-path src-tauri/Cargo.toml --features integration --test ssh
#   scripts/dev-sshd.sh stop
#
# Needs OpenSSH (sshd, ssh-keygen). Everything lives in a temp directory;
# your own ~/.ssh is not touched. sshd must be able to read the authorized
# key, so this runs it as the current user (or via sudo when available, which
# is what CI does).
set -euo pipefail

dir="${ROWBOAT_SSHD_DIR:-${TMPDIR:-/tmp}/rowboat-test-sshd}"
port="${ROWBOAT_TEST_SSH_PORT:-2222}"
sudo=""
if [ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
  sudo="sudo"
fi

if [ "${1:-}" = "stop" ]; then
  [ -f "$dir/sshd.pid" ] && $sudo kill "$(cat "$dir/sshd.pid")" 2>/dev/null || true
  exit 0
fi

sshd_bin="$(command -v sshd || echo /usr/sbin/sshd)"
mkdir -p "$dir"
[ -f "$dir/host_key" ] || ssh-keygen -q -t ed25519 -N '' -f "$dir/host_key"
[ -f "$dir/user_key" ] || ssh-keygen -q -t ed25519 -N '' -f "$dir/user_key"
cp "$dir/user_key.pub" "$dir/authorized_keys"
chmod 600 "$dir/user_key" "$dir/authorized_keys"

cat > "$dir/sshd_config" <<CONFIG
Port $port
ListenAddress 127.0.0.1
HostKey $dir/host_key
PidFile $dir/sshd.pid
AuthorizedKeysFile $dir/authorized_keys
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
AllowTcpForwarding yes
StrictModes no
CONFIG

[ -f "$dir/sshd.pid" ] && $sudo kill "$(cat "$dir/sshd.pid")" 2>/dev/null || true
[ -d /run/sshd ] || $sudo mkdir -p /run/sshd
$sudo "$sshd_bin" -f "$dir/sshd_config" -E "$dir/sshd.log"

for _ in $(seq 1 50); do
  if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    echo "export ROWBOAT_TEST_SSH_PORT=$port"
    echo "export ROWBOAT_TEST_SSH_USER=$(id -un)"
    echo "export ROWBOAT_TEST_SSH_KEY=$dir/user_key"
    exit 0
  fi
  sleep 0.1
done
echo "sshd did not start; see $dir/sshd.log" >&2
exit 1
