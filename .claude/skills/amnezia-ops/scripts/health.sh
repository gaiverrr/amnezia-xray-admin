#!/usr/bin/env bash
# Read-only health check of both hops (bridge + egress). Safe to run anytime.
set -u

ok()   { printf '\033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '\033[33m!\033[0m %s\n' "$*"; }
fail() { printf '\033[31m✗\033[0m %s\n' "$*"; }

require_alias() {
  ssh -o BatchMode=yes -o ConnectTimeout=5 "$1" true 2>/dev/null || {
    fail "SSH alias '$1' unreachable — check ~/.ssh/config"
    return 1
  }
}

echo "=== Bridge (yc-vm / 81.26.189.136) ==="
if require_alias yc-vm; then
  if ssh yc-vm 'sudo systemctl is-active xray' 2>/dev/null | grep -q active; then
    ok "xray service active"
  else
    fail "xray service NOT active"
  fi

  listen=$(ssh yc-vm 'sudo ss -tlnp 2>/dev/null | grep ":443 "')
  if [ -n "$listen" ]; then ok "listening on :443"; else fail "NOT listening on :443"; fi

  err_tail=$(ssh yc-vm 'sudo tail -n 5 /var/log/xray/error.log 2>/dev/null')
  reject=$(echo "$err_tail" | grep -iE 'reject|fail|error' | grep -v '^$' || true)
  if [ -z "$reject" ]; then ok "no recent errors in xray error.log"
  else warn "error.log tail contains warnings:"; echo "$err_tail" | sed 's/^/    /'
  fi

  clients=$(ssh yc-vm 'sudo jq ".inbounds[0].settings.clients | length" /usr/local/etc/xray/config.json' 2>/dev/null)
  ok "$clients clients registered on bridge"
fi

echo ""
echo "=== Egress (vps-vpn / 103.231.72.109) ==="
if require_alias vps-vpn; then
  if ssh vps-vpn 'sudo systemctl is-active xray' 2>/dev/null | grep -q active; then
    ok "xray-egress service active"
  else
    fail "xray-egress service NOT active"
  fi

  listen=$(ssh vps-vpn 'sudo ss -tlnp 2>/dev/null | grep ":8444 "')
  if [ -n "$listen" ]; then ok "listening on :8444"; else fail "NOT listening on :8444"; fi

  nginx_active=$(ssh vps-vpn 'sudo systemctl is-active nginx' 2>/dev/null)
  if [ "$nginx_active" = "active" ]; then ok "nginx active"; else fail "nginx NOT active"; fi

  nginx_listen=$(ssh vps-vpn 'sudo ss -tlnp 2>/dev/null | grep "127.0.0.1:9443"')
  if [ -n "$nginx_listen" ]; then ok "nginx self-steal on 127.0.0.1:9443"; else fail "nginx NOT on 127.0.0.1:9443"; fi

  cert_days=$(ssh vps-vpn 'sudo bash -c "openssl x509 -in /etc/letsencrypt/live/yuriy-vps.duckdns.org/fullchain.pem -noout -enddate 2>/dev/null | cut -d= -f2"' 2>/dev/null)
  if [ -n "$cert_days" ]; then
    expiry_epoch=$(date -j -f "%b %e %H:%M:%S %Y %Z" "$cert_days" +%s 2>/dev/null || date -d "$cert_days" +%s 2>/dev/null)
    now_epoch=$(date +%s)
    if [ -n "$expiry_epoch" ]; then
      days_left=$(( (expiry_epoch - now_epoch) / 86400 ))
      if [ "$days_left" -lt 14 ]; then
        warn "LE cert expires in $days_left days"
      else
        ok "LE cert valid for $days_left days"
      fi
    fi
  else
    warn "could not read LE cert expiry"
  fi
fi

echo ""
echo "=== Bridge → Egress connectivity ==="
bridge_to_egress=$(ssh yc-vm 'nc -zv -w 5 103.231.72.109 8444 2>&1' 2>/dev/null || true)
if echo "$bridge_to_egress" | grep -qi succeeded; then
  ok "bridge can reach egress:8444"
else
  fail "bridge cannot reach egress:8444 — $bridge_to_egress"
fi
