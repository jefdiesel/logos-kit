#!/usr/bin/env bash
# S4 exit proof, through the `logos-kit` CLI against the standalone sequencer
# (e2e/standalone.sh), dev-mode proofs:
#   - the faucet reaches `funded` (public target) and funds + shields a private one;
#   - a demo token is created, then moved on every route: public → public,
#     shield, private → private, unshield;
#   - native unshield; the builtin token program's header checks out;
#   - an encrypted backup restores with its password.
# LEZ v0.3.1 dropped the token, ATA and AMM programs from its builtins, so the
# token stages run only on a chain that has them: LK_E2E_TOKENS=1.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export RISC0_DEV_MODE=1
(cd "$ROOT" && cargo build -q -p logos-kit-cli)
LK="$ROOT/target/debug/logos-kit"

export LOGOS_KIT_HOME="$(mktemp -d)"
RESTORED="$(mktemp -d)"
export LOGOS_KIT_PASSWORD=e2e-password LOGOS_KIT_ZONE=lez-local SUPPRESS_VERBOSE_PRINTS=1
trap 'rm -rf "$LOGOS_KIT_HOME" "$RESTORED" "${STRANGER:-}"' EXIT
# LEZ debug genesis account (vendor/lez/Justfile `wallet-import-test-accounts`),
# used here as the faucet's funded key (the e2e GenesisSupply backend).
export LK_GENESIS_KEY=7f273098f25b71e6c005a9519f2678da8d1c7f01f6a27778e2d9948abdf901fb
field() { tail -1 | python3 -c "import sys,json; v=json.load(sys.stdin); print(eval('v'+sys.argv[1]))" "$1"; }
check() { [[ "$2" == "$3" ]] || { echo "FAIL $1: got $2, want $3" >&2; exit 1; }; echo "ok   $1 = $2"; }
TOKENS=${LK_E2E_TOKENS:-0}

"$LK" init --json 2>/dev/null >/dev/null
PUB=$("$LK" account new --json | field "['accountId']")
PUB2=$("$LK" account new --json | field "['accountId']")
PRIV=$("$LK" account new --private --json | field "['accountId']")
PRIV2=$("$LK" account new --private --json | field "['accountId']")

# Faucet: public target → funded.
OUT=$("$LK" faucet "$PUB" --key-env LK_GENESIS_KEY --drop 2000000000 --yes --json)
check "faucet public status" "$(echo "$OUT" | field "['status']")" funded
check "PUB native" "$("$LK" balance "$PUB" --json | field "['balance']")" 2000000000
# Rate limit is per process here (a service keeps it); a second claim works.
# Faucet: private target → funds PUB2, then shields into PRIV.
"$LK" faucet "$PRIV" --via "$PUB2" --key-env LK_GENESIS_KEY --drop 5000 --yes --json >/dev/null
check "PRIV native (faucet → shield)" "$("$LK" balance "$PRIV" --json | field "['balance']")" 5000

PUB3=$("$LK" account new --json | field "['accountId']")
if [[ "$TOKENS" == 1 ]]; then
# Demo token: a new definition account; all supply to PUB.
DEF=$("$LK" token create --name DEMO --supply 1000000 --holder "$PUB" --yes --json | field "['definition']")
check "PUB DEMO" "$("$LK" balance "$PUB" --token "$DEF" --json | field "['balance']")" 1000000

"$LK" send --from "$PUB" --to "$PUB2" --token "$DEF" --amount 100 --yes --json >/dev/null
check "public token send" "$("$LK" balance "$PUB2" --token "$DEF" --json | field "['balance']")" 100

"$LK" shield --from "$PUB" --to "$PRIV2" --token "$DEF" --amount 50 --yes --json >/dev/null
check "token shield" "$("$LK" balance "$PRIV2" --token "$DEF" --json | field "['balance']")" 50

PRIV3=$("$LK" account new --private --json | field "['accountId']")
"$LK" send --from "$PRIV2" --to "$PRIV3" --token "$DEF" --amount 20 --yes --json >/dev/null
check "private token send (from)" "$("$LK" balance "$PRIV2" --token "$DEF" --json | field "['balance']")" 30
check "private token send (to)" "$("$LK" balance "$PRIV3" --token "$DEF" --json | field "['balance']")" 20

"$LK" deshield --from "$PRIV3" --to "$PUB3" --token "$DEF" --amount 5 --yes --json >/dev/null
check "token unshield" "$("$LK" balance "$PUB3" --token "$DEF" --json | field "['balance']")" 5
fi

"$LK" deshield --from "$PRIV" --to "$PUB3" --amount 1234 --yes --json >/dev/null
check "native unshield" "$("$LK" balance "$PUB3" --json | field "['balance']")" 1234
check "native private left" "$("$LK" balance "$PRIV" --json | field "['balance']")" 3766

# Private payment to someone else's keys: wallet B shares its private
# account's keys; A pays them; B finds the note by syncing.
B_HOME="$(mktemp -d)"; trap 'rm -rf "$LOGOS_KIT_HOME" "$RESTORED" "$B_HOME"' EXIT
LOGOS_KIT_HOME="$B_HOME" "$LK" init --json 2>/dev/null >/dev/null
B_PRIV=$(LOGOS_KIT_HOME="$B_HOME" "$LK" account new --private --json | field "['accountId']")
LOGOS_KIT_HOME="$B_HOME" "$LK" account keys "$B_PRIV" > "$B_HOME/b.keys"
"$LK" send --from "$PRIV" --to-keys "$B_HOME/b.keys" --amount 700 --yes --json >/dev/null
check "native private left after paying B" "$("$LK" balance "$PRIV" --json | field "['balance']")" 3066
LOGOS_KIT_HOME="$B_HOME" "$LK" sync --json >/dev/null
B_GOT=$(LOGOS_KIT_HOME="$B_HOME" "$LK" account list --json | tail -1 | python3 -c "
import sys,json
print(len([a for a in json.load(sys.stdin) if a['kind']=='private']))")
echo "info B private accounts after sync: $B_GOT"
B_SUM=0
for acct in $(LOGOS_KIT_HOME="$B_HOME" "$LK" account list --json | tail -1 | python3 -c "
import sys,json
print(' '.join(a['accountId'] for a in json.load(sys.stdin) if a['kind']=='private'))"); do
  B_SUM=$((B_SUM + $(LOGOS_KIT_HOME="$B_HOME" "$LK" balance "$acct" --json | field "['balance']")))
done
check "B received privately" "$B_SUM" 700

if [[ "$TOKENS" == 1 ]]; then
# Public token sends land in the recipient's token account (ATA) for that
# token, created on arrival; tokens there are sent on through the ATA program.
PUB4=$("$LK" account new --json | field "['accountId']")
"$LK" faucet "$PUB4" --key-env LK_GENESIS_KEY --drop 1000000000 --yes --json >/dev/null
ATA4=$("$LK" token ata "$PUB4" "$DEF" --json | field "['ata']")
"$LK" send --from "$PUB" --to "$PUB4" --token "$DEF" --amount 7 --yes --json >/dev/null
IN_ATA=$("$LK" token list --json | tail -1 | python3 -c "import sys,json; print(sum(int(h['amount']) for h in json.load(sys.stdin) if h['account']=='$PUB4' and h['via']=='ata' and h['holder']=='$ATA4'))")
check "lands in the recipient's ATA" "$IN_ATA" 7
"$LK" send --from "$PUB4" --to "$PUB2" --token "$DEF" --amount 3 --yes --json >/dev/null
check "ATA token send" "$("$LK" balance "$PUB2" --token "$DEF" --json | field "['balance']")" 103

N=$("$LK" token list --json | tail -1 | python3 -c "import sys,json; print(len([h for h in json.load(sys.stdin) if h['definition']=='$DEF']))")
check "token list holdings" "$N" 6

# Stage T: tokens nobody told the wallet about. A stranger (another wallet)
# creates two tokens and sends them to PUB; they land in PUB's token
# accounts. The wallet finds them by reading blocks: one Unknown, one Spam
# (its name is airdrop bait). Then add by ID, hide, and one account holding
# three different tokens.
STRANGER="$(mktemp -d)"
SLK() { LOGOS_KIT_HOME="$STRANGER" "$LK" "$@"; }
SLK init --json >/dev/null 2>&1
S1=$(SLK account new --json | field "['accountId']")
SLK faucet "$S1" --key-env LK_GENESIS_KEY --drop 1000000000 --yes --json >/dev/null
SDEF=$(SLK token create --name "Payroll Coin" --supply 5000 --holder "$S1" --yes --json | tail -1 | python3 -c "import sys,json; print(json.load(sys.stdin)['definition'])")
S2=$(SLK account new --json | field "['accountId']")
SLK faucet "$S2" --key-env LK_GENESIS_KEY --drop 1000000000 --yes --json >/dev/null
BAIT=$(SLK token create --name "Claim free gift" --supply 9999 --holder "$S2" --yes --json | tail -1 | python3 -c "import sys,json; print(json.load(sys.stdin)['definition'])")
SLK send --from "$S1" --to "$PUB" --token "$SDEF" --amount 42 --yes --json >/dev/null
SLK send --from "$S2" --to "$PUB" --token "$BAIT" --amount 1 --yes --json >/dev/null
tier_of() { "$LK" token list --all --json | tail -1 | python3 -c "import sys,json; t=[h['tier'] for h in json.load(sys.stdin) if h['definition']=='$1']; print(t[0] if t else 'missing')"; }
check "a stranger's token shows as Unknown" "$(tier_of "$SDEF")" unknown
check "airdrop bait shows as Spam" "$(tier_of "$BAIT")" spam
REASON=$("$LK" token list --spam --json | tail -1 | python3 -c "import sys,json; print([h.get('spamReason') for h in json.load(sys.stdin) if h['definition']=='$BAIT'][0])")
check "spam reason" "$REASON" "Name looks like an airdrop offer"
DEFAULT_N=$("$LK" token list --json | tail -1 | python3 -c "import sys,json; print(len([h for h in json.load(sys.stdin) if h['definition'] in ('$SDEF','$BAIT')]))")
check "Unknown and Spam stay out of the default list" "$DEFAULT_N" 0
"$LK" token add "$SDEF" --decimals 2 --yes --json >/dev/null
check "added by ID" "$(tier_of "$SDEF")" added
"$LK" token hide "$SDEF"
check "hidden" "$(tier_of "$SDEF")" hidden
"$LK" token unhide "$SDEF"
check "shown again (still added, never verified)" "$(tier_of "$SDEF")" added
KINDS=$("$LK" token list --all --json | tail -1 | python3 -c "import sys,json; print(len({h['definition'] for h in json.load(sys.stdin) if h['account']=='$PUB'}))")
check "one account holds three different tokens" "$KINDS" 3
rm -rf "$STRANGER"

STATUS=$("$LK" program token --json | field "['status']")
[[ "$STATUS" == verified_local ]] || { echo "FAIL token program status $STATUS" >&2; exit 1; }
echo "ok   builtin token program header = $STATUS"
fi

"$LK" backup export "$RESTORED/backup.json"
LOGOS_KIT_HOME="$RESTORED/home" "$LK" backup import "$RESTORED/backup.json" >/dev/null
check "restored PUB native" "$(LOGOS_KIT_HOME="$RESTORED/home" "$LK" balance "$PUB" --json | field "['balance']")" "$("$LK" balance "$PUB" --json | field "['balance']")"
if [[ "$TOKENS" == 1 ]]; then
  check "restored PUB DEMO" "$(LOGOS_KIT_HOME="$RESTORED/home" "$LK" balance "$PUB" --token "$DEF" --json | field "['balance']")" 999843
  echo "OK: S4 faucet + demo token on every route + backup (dev-mode proofs)"
else
  echo "OK: S4 faucet, native and private routes + backup (dev-mode proofs; LK_E2E_TOKENS=1 adds the token stages)"
fi
