# Bot Manager — local web app for the PlayTak bot ladder

A local React + MUI web app to configure and run the 6-bot ladder (Novice / Intermediate / Advanced
× 5×5 / 6×6) without hand-editing the PowerShell scripts. It reuses `run-gembot.ps1` as the launch
layer, so the proven logic (BelowNormal priority, restart loop, env-var mapping) stays the single
source of truth — the app just owns the config and process lifecycle.

- **Backend** (`server/`): Express + TypeScript, binds `127.0.0.1:4317`. Owns the config JSON, the
  bot processes, and the game database. No auth (local only).
- **Frontend** (`web/`): React + Vite + MUI, `127.0.0.1:5173`, proxies `/api` to the backend.
- **Config**: `bot-manager-config.json` (git-ignored; seeded with sensible per-level defaults).
- **Game database**: `D:\TakBotManager\bot-games.db` (SQLite) — see below.

## Game database

Every finished game is written to a SQLite database rather than to loose files, so the history
survives indefinitely and can be queried. It lives on **D:** by default (far more free space than
C:); override with the `BOT_MANAGER_DATA_DIR` environment variable, and if neither D: nor the
override is writable it falls back to `bot-manager-app/data/`.

Each row holds the game's metadata (opponent, the bot's colour, PlayTak game id, result string,
win/loss/draw, timestamps, move count, board size, which server it was played on) **plus a snapshot
of the bot's configuration at launch** and the captured log, gzipped (~5× smaller). Nothing is
pruned — a game costs a few KB, so 100k games is well under a gigabyte.

**Opponent ratings** aren't available over the PlayTak telnet protocol, so a finished game starts
with its rating unknown and a background worker fills it in from the HTTP API: first
`/v1/games-history/<id>` (both players' ratings at the time, validated against the player names so a
game number from a different server can't be mistaken for yours), falling back to
`/v1/ratings/<opponent>` for games whose id was never captured. Guests have no rating and stay
blank.

The logs that existed before the database (`game-logs/`) were imported automatically on first run —
they're left on disk untouched, but nothing new is written there. They carry no configuration
snapshot, so they never match a live bot's settings in the ELO plots below.

## Start it (double-click)

Double-click **`startBotManager.cmd`** in this folder. It installs dependencies on first run, starts
the backend + frontend, and opens the UI at http://127.0.0.1:5173. No terminal needed.

From a shell instead:

```powershell
cd C:\Users\lance\Desktop\Tak\Bots\topaz-tak\bot-manager-app
.\startBotManager.ps1              # start (auto-installs deps if missing)
.\startBotManager.ps1 -Install     # force a fresh reinstall, then start
```

Two dev-server windows open (backend + frontend). Close them (or Ctrl-C in each) to stop the app.
Closing the **backend** window also stops any bots the app launched.

## Using the app

1. **Server target** — pick **Local / Beta / Prod** at the top of Global settings (sets host+port:
   Local `localhost:10000`, Beta `beta.playtak.com:10002`, Prod `playtak.com:10000`).
2. **Password template** — `{user}` is replaced per bot (e.g. `{user}_S3cret!` → `AdvancedBot6_S3cret!`).
3. **Rated** — off = unrated (testing), on = rated.
4. Adjust per-bot **Search depth / Eval gap (Min≤Std≤Max) / Bias slope / Move pool** (and optional
   Advanced: Abort depth, Time, Hash, Priority). Search depth is a fixed iterative-deepening depth
   (e.g. Novice 4, Intermediate 6, Advanced 8) — the same regardless of board complexity, with a
   12s/move time backstop so a hairy position never flags. With **Link 5×5/6×6** on, editing a level
   updates both sizes.
5. **Save** (changes must be saved before launching — Start/Restart are disabled while unsaved).
6. **Start all** / per-bot **Start / Stop / Restart**. Status dots: green = connected, blue =
   running/connecting, purple = running outside the app (orphan), grey = stopped. The **logs**
   button (☰) opens live output per bot.

## Logs panel

The header shows two box plots on a shared axis: the opponent ELOs the bot has **won** against and
the ones it has **lost** against. They only count games played under the bot's *current* saved
configuration, so changing any tunable (or an advanced override) starts a fresh sample — a config
that's never been played shows an empty state. The game list itself is never filtered this way.

The **History** tab lists the newest 30 saved games with their result string (`0-F`, `R-0`, `½-½`).
Selecting one shows its log above a sticky footer with the configuration that game was played under
and an **Open in PTN Ninja** button (disabled for games on a server without a public game-history
page, e.g. a local one). On phones the log panel is hidden and tapping a game opens it full-screen
instead — there isn't room for both side by side.

## Running on Beta (rated)

Accounts must be registered on beta **and** bot-flagged by an admin (`sudo bot <name>`) first; each
account's password should be `<username><your-suffix>` (see BOT_COMMANDS.md).

1. Double-click `startBotManager.cmd`.
2. Click **Beta**. Set **Password template** to `{user}<your-suffix>`.
3. Pre-flight: leave **Rated off**, Save, and **Start** one bot (e.g. AdvancedBot6). Open its logs
   and confirm it connects, logs in, and posts a seek (no desync on move 1 — beta must have Double
   Black Stack deployed).
4. When happy: **Stop** it, turn **Rated on**, **Save**, **Start all**.

## Ports

Backend `127.0.0.1:4317`, frontend `127.0.0.1:5173` (both clear of the telnet ports 10000/10002).
