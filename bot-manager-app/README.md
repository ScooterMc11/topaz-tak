# Bot Manager — local web app for the PlayTak bot ladder

A local React + MUI web app to configure and run the 6-bot ladder (Novice / Intermediate / Advanced
× 5×5 / 6×6) without hand-editing the PowerShell scripts. It reuses `run-gembot.ps1` as the launch
layer, so the proven logic (BelowNormal priority, restart loop, env-var mapping) stays the single
source of truth — the app just owns the config and process lifecycle.

- **Backend** (`server/`): Express + TypeScript, binds `127.0.0.1:4317`. Owns the config JSON and
  the bot processes. No auth (local only).
- **Frontend** (`web/`): React + Vite + MUI, `127.0.0.1:5173`, proxies `/api` to the backend.
- **Config**: `bot-manager-config.json` (git-ignored; seeded with sensible per-level defaults).

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
