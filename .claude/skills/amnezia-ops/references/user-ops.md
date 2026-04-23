# User operations (new bridge)

For the NEW double-hop bridge. The legacy Amnezia VPN is still managed by the Telegram bot on `vps-vpn`.

## Add a user

Helper: `.claude/skills/amnezia-ops/scripts/bridge-user-add.sh <name>`.

Manual equivalent:
```bash
NAME="..."
UUID=$(cat /proc/sys/kernel/random/uuid)  # or uuidgen
ssh yc-vm "sudo cp /usr/local/etc/xray/config.json /usr/local/etc/xray/config.json.bak-\$(date +%s) && \
  sudo jq --arg uuid '$UUID' --arg email '${NAME}@vpn' \
    '.inbounds[0].settings.clients += [{id: \$uuid, email: \$email}]' \
    /usr/local/etc/xray/config.json | sudo tee /usr/local/etc/xray/config.json.new > /dev/null && \
  sudo mv /usr/local/etc/xray/config.json.new /usr/local/etc/xray/config.json && \
  sudo systemctl restart xray"
sleep 2
ssh yc-vm 'sudo tail -3 /var/log/xray/error.log'
# Then regenerate URL — see below.
```

## Regenerate URL for existing user

Helper: `.claude/skills/amnezia-ops/scripts/bridge-url.sh <name>`.

Manual equivalent: fetch the user's UUID from bridge config, fetch public params, construct URL:

```bash
NAME="..."
UUID=$(ssh yc-vm "sudo jq -r '.inbounds[0].settings.clients[] | select(.email==\"${NAME}@vpn\") | .id' /usr/local/etc/xray/config.json")
PBK=$(ssh yc-vm 'sudo cat /usr/local/etc/xray/reality-public-key')
SID=$(ssh yc-vm "sudo jq -r '.inbounds[0].streamSettings.realitySettings.shortIds[0]' /usr/local/etc/xray/config.json")
PATH_=$(ssh yc-vm "sudo jq -r '.inbounds[0].streamSettings.xhttpSettings.path' /usr/local/etc/xray/config.json")
SNI=$(ssh yc-vm "sudo jq -r '.inbounds[0].streamSettings.realitySettings.serverNames[0]' /usr/local/etc/xray/config.json")

PATH_ENC=$(printf '%s' "$PATH_" | sed 's|/|%2F|g')
URL="vless://${UUID}@81.26.190.206:443?encryption=none&type=xhttp&path=${PATH_ENC}&security=reality&sni=${SNI}&fp=chrome&pbk=${PBK}&sid=${SID}#${NAME}"
echo "$URL"
qrencode -t ANSIUTF8 "$URL"  # if qrencode installed locally
```

## Remove a user

Helper: `.claude/skills/amnezia-ops/scripts/bridge-user-remove.sh <name>`.

Manual:
```bash
NAME="..."
ssh yc-vm "sudo cp /usr/local/etc/xray/config.json /usr/local/etc/xray/config.json.bak-\$(date +%s) && \
  sudo jq --arg email '${NAME}@vpn' \
    '.inbounds[0].settings.clients |= map(select(.email != \$email))' \
    /usr/local/etc/xray/config.json | sudo tee /usr/local/etc/xray/config.json.new > /dev/null && \
  sudo mv /usr/local/etc/xray/config.json.new /usr/local/etc/xray/config.json && \
  sudo systemctl restart xray"
```

## List all bridge users

```bash
ssh yc-vm 'sudo jq ".inbounds[0].settings.clients[] | .email" /usr/local/etc/xray/config.json'
```

## Reality key rotation (paranoia / suspected compromise)

Since every client URL embeds the **bridge public key** (pbk), rotating keys requires re-issuing URLs to all users. ~15 users on this install = acceptable.

Bridge-side rotation:
```bash
# 1. On bridge, generate a fresh x25519 pair
ssh yc-vm 'xray x25519'
# → capture PrivateKey and Password (PublicKey) values.

# 2. Patch bridge config: realitySettings.privateKey = new PrivateKey; shortIds optional rotate too
# 3. Update sidecar: echo NEW_PUB | sudo tee /usr/local/etc/xray/reality-public-key
# 4. systemctl restart xray
# 5. For every user, regenerate their URL (same UUID, new pbk).
```

Egress-side rotation is transparent to clients — only the bridge outbound block needs updating:
```bash
# 1. On egress, generate fresh x25519
ssh vps-vpn 'xray x25519'
# 2. Patch egress config realitySettings.privateKey on vps-vpn, restart xray.
# 3. Patch bridge's outbound foreign-egress block:
#    streamSettings.realitySettings.publicKey = new pub
#    Then restart xray on bridge.
```

## Bot status note

As of the latest ops snapshot, the Telegram bot on `vps-vpn` talks ONLY to the legacy Amnezia Docker container. It cannot add/remove users on the new bridge. When the bridge-aware bot is deployed to `yc-vm`, this file's "Add a user" section can be simplified to "tell Yuriy to use /add in Telegram".
