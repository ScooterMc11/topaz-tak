# Bot Ladder - Command Reference

Copy-paste commands for running the 6-bot skill ladder (Novice / Intermediate / Advanced x 5x5 / 6x6).
Run these from `C:\Users\lance\Desktop\Tak\Bots\topaz-tak` in PowerShell. Each bot runs on THIS machine
and connects out to the chosen server; each gets its own auto-restarting window.

## Stages (host / telnet port)

| Stage | Host | Port | When |
|---|---|---|---|
| Local | `localhost` | `10000` | now (local fork must be running) |
| Beta | `beta.playtak.com` | `10002` | after the DBS change is on `dev` + beta redeploys |
| Prod | `playtak.com` | `10000` | after it merges to `main` |

## Ladder configs (baked into `run-ladder.ps1`)

Weakening is a DYNAMIC eval-gap (Min/Std/Max cp): gap flexes with the bot's eval — Min at <=-50cp
(losing => strongest), Std at 0cp (even), Max at >=+50cp (winning => weakest). Same triples both sizes.

| Bot | Size | Nodes | AbortDepth | GapMin/Std/Max | MovePool | Time control |
|---|---|---|---|---|---|---|
| AdvancedBot6     | 6 | 500000 | 1 | 20/30/40  | 16 | 10+n (600s) |
| IntermediateBot6 | 6 | 500000 | 1 | 40/50/60  | 16 | 10+n (600s) |
| NoviceBot6       | 6 | 500000 | 1 | 60/80/100 | 16 | 10+n (600s) |
| AdvancedBot5     | 5 | 400000 | 1 | 20/30/40  | 16 | 7+n (420s)  |
| IntermediateBot5 | 5 | 400000 | 1 | 40/50/60  | 16 | 7+n (420s)  |
| NoviceBot5       | 5 | 400000 | 1 | 60/80/100 | 16 | 7+n (420s)  |

The gap (a.k.a. "drop rate") prunes root moves more than `gap` cp below the best, then picks among the
survivors with a linear rank bias (`BiasSlope`, shared across levels for now, decimal ok): 0 = uniform,
higher tilts toward the WORSE eligible moves (worst:best odds = 1 + BiasSlope). `-AlwaysAllow N` sets an
absolute floor: any move scoring >= N cp stays eligible even if it falls outside the gap band (widens the
pool only; never overrides a solved/decisive position or plays a losing move). Large default = disabled. Over a pool-16 MultiPV (max 16). White forces a
random corner; Black's first reply picks an archetype (now equal-weighted). BiasSlope is per-level in
run-ladder ($b.Slope); currently 8 for both sizes. Nodes per-size (6x6=50k, 5x5=40k); levels differ by gap. The dynamic gap makes the bot ease off when winning and bear down when losing,
flexing around a roughly static level. Values are MANUAL (slatebot calibration was judged
unrepresentative) - tune the triples live off ratings/feel (higher = weaker). `+n` = increment scales
with move number (base increment 1s).

---

## Build (do this after any code change)

```powershell
cd C:\Users\lance\Desktop\Tak\Bots\topaz-tak
git branch --show-current            # expect: playtak-bots
cargo build --release                # -> target\release\topaz.exe
```

## Local (fork on this machine; accounts in local players.db, pw 'password')

```powershell
# All 6, UNRATED (test): accept a seek in the browser to play one
.\run-ladder.ps1

# All 6, RATED
.\run-ladder.ps1 -Rated

# Just the 6x6 bots
.\run-ladder.ps1 -Only AdvancedBot6,IntermediateBot6,NoviceBot6

# A single bot (unrated)
.\run-gembot.ps1 -User AdvancedBot6 -Pass password -Size 6
```

## Beta (beta.playtak.com:10002)

Accounts must be registered on beta AND bot-flagged by an admin (`sudo bot <name>`) first. `-Pass` is a
template: `{user}` is replaced by each bot's username, so set each account's password to
`<username><your-suffix>` and pass the template once. Replace `<suffix>` below with your real suffix.

```powershell
# Pre-flight ONE bot UNRATED first - verify the 2a1 opening + a full game with no desync
.\run-gembot.ps1 -User AdvancedBot6 -Pass "AdvancedBot6<suffix>" -Size 6 -ServerHost beta.playtak.com -Port 10002

# Then launch all 6, RATED (per-bot passwords via the {user} template)
.\run-ladder.ps1 -ServerHost beta.playtak.com -Port 10002 -Pass "{user}<suffix>" -Rated
```

## Prod (playtak.com:10000) - same as beta but different host/port

```powershell
# Pre-flight one bot UNRATED
.\run-gembot.ps1 -User AdvancedBot6 -Pass "AdvancedBot6<suffix>" -Size 6 -ServerHost playtak.com -Port 10000

# All 6, RATED
.\run-ladder.ps1 -ServerHost playtak.com -Port 10000 -Pass "{user}<suffix>" -Rated
```

---

## Common tweaks

```powershell
# Re-tune ONE level after watching its live rating (raise the gap triple to weaken, lower to strengthen).
# Relaunch just that bot with the override; e.g. make NoviceBot6 a bit stronger (20/40/60 -> 15/30/45):
.\run-gembot.ps1 -User NoviceBot6 -Pass "NoviceBot6<suffix>" -Size 6 -EvalGapMin 15 -EvalGapStd 30 -EvalGapMax 45 -MovePool 16 -ServerHost beta.playtak.com -Port 10002 -Rated

# Run a single bot at FULL strength (EvalGap 0 is the default => no weakening):
.\run-gembot.ps1 -User AdvancedBot5 -Pass "AdvancedBot5<suffix>" -Size 5 -ServerHost beta.playtak.com -Port 10002 -Rated

# Override the time control for one bot (e.g. 6x6 at 5+n instead of 10+n):
.\run-gembot.ps1 -User AdvancedBot6 -Pass "AdvancedBot6<suffix>" -Size 6 -Time 300 -ServerHost beta.playtak.com -Port 10002 -Rated
```

## Local account setup (first time only)

```bash
# In playtak-api, create the 6 local accounts with isbot=1 (see scripts/development/add_user.sh).
# On beta/prod you instead REGISTER the accounts on the site, then an admin runs `sudo bot <name>`.
```

## Notes

- **Stop a bot:** close its PowerShell window (or Ctrl-C). Each window has a restart loop, so the bot
  also auto-recovers from crashes/telnet drops.
- **Telnet is unencrypted on every stage** - the login password goes over the wire in plaintext. Use
  throwaway passwords for the bot accounts, not ones you reuse elsewhere.
- **Don't point these bots at a server that lacks Double Black Stack** (they desync on move 1). Only run
  against beta/prod once the DBS change is deployed there.
- Colors: each bot seeks with `-A` (accepts either color) and, when Black, plays the weighted archetype
  first reply regardless of level.
