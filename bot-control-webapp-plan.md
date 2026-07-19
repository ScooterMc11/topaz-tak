# Bot Control Web App — Planning Doc

A small, local, React + MUI web app to control the 6-bot PlayTak ladder (start/stop/restart,
live status, and per-bot + global config with sliders) without hand-editing the PowerShell
scripts. Local-only, single-user, no auth.

Status: **PLAN — awaiting approval before implementation.**

---

## 1. Goal & scope

### Goal
Replace hand-editing of `run-ladder.ps1`'s `$ladder` table (and re-launching bots by hand) with a
clean local web UI that:
- Adjusts every per-bot tunable (Nodes, GapMin/Std/Max, BiasSlope, MovePool) and the global
  settings (ServerHost, Port, password template, Rated, StaggerSeconds) using MUI sliders for the
  numeric values, grouped by level and board size, with the `GapMin <= GapStd <= GapMax`
  invariant enforced in the UI.
- Starts / stops / restarts all bots, and individual bots.
- Shows live status (which bots are up / connected).
- Persists config to disk so choices survive app and machine restarts.

### In scope
- A local backend (Node + Express) that owns config (JSON on disk) and manages bot processes.
- A React + MUI frontend served locally, talking to the backend over `127.0.0.1`.
- Reuse of the **proven** per-bot launch logic in `run-gembot.ps1` as the execution layer.

### Non-goals (explicit)
- **No** changes to the Rust engine, telnet protocol, or the opening/gap/bias logic itself.
- **No** remote access, multi-user, or authentication. Binds to loopback only.
- **No** replacement of the engine's env-var contract — the app produces the same
  `run-gembot.ps1` invocations a human would.
- **No** cloud, no packaging/installer, no auto-start-on-boot in the first cut (listed as an open
  question).
- **No** editing of a bot's fixed identity (its name and board Size are fixed per bot).
- **Not** a game viewer / analysis UI — control panel only.

---

## 2. Current state (from the scripts — source of truth)

### The 6 bots
`AdvancedBot6, IntermediateBot6, NoviceBot6` (Size 6) and
`AdvancedBot5, IntermediateBot5, NoviceBot5` (Size 5).

### Per-bot config lives in `run-ladder.ps1`'s `$ladder` table
Each entry: `User, Size, Nodes, GapMin, GapStd, GapMax, Slope (BiasSlope), Pool (MovePool)`.
Current values:

| Bot | Size | Nodes | GapMin/Std/Max | BiasSlope | MovePool |
|---|---|---|---|---|---|
| AdvancedBot6     | 6 | 50000 | 20/30/40  | 8 | 16 |
| IntermediateBot6 | 6 | 50000 | 40/50/60  | 8 | 16 |
| NoviceBot6       | 6 | 50000 | 60/80/100 | 8 | 16 |
| AdvancedBot5     | 5 | 40000 | 20/30/40  | 8 | 16 |
| IntermediateBot5 | 5 | 40000 | 40/50/60  | 8 | 16 |
| NoviceBot5       | 5 | 40000 | 60/80/100 | 8 | 16 |

Global launch params (also `run-ladder.ps1`): `ServerHost` (default `localhost`), `Port`
(default `10000`), `Pass` (a template; `{user}` → each bot's username, e.g. `"{user}_S3cret!"`),
`Rated` (switch), `Only` (subset filter), `StaggerSeconds` (default `8`).

### The per-bot launch surface lives in `run-gembot.ps1`
It takes: `User, Pass, Size, Nodes, AbortDepth (1), Time (5x5=420, 6x6=600), EvalGap (0),
EvalGapMin/Std/Max, BiasSlope, Temperature (0, legacy), MovePool, ServerHost, Port, Rated,
HashMb (48), Priority (BelowNormal)`. It maps these to `PLAYTAK_*` env vars, then runs
`target\release\topaz.exe playtak` in a `while ($true)` **restart loop**, dropping the child to
`BelowNormal` priority. It also sets the fixed opening/time knobs: `PLAYTAK_OPENING=1` (Double
Black Stack), `PLAYTAK_INC=1`, `PLAYTAK_INC_SCALES=1`, `PLAYTAK_COLOR=A`,
`PLAYTAK_UNRATED = Rated ? 0 : 1`.

### How launch/stop/status work today
- **Launch:** `run-ladder.ps1` loops the table and, for each bot, calls
  `Start-Process powershell -ArgumentList (-NoExit -File run-gembot.ps1 -User ... -Nodes ... )`,
  i.e. **one detached PowerShell window per bot**, staggered by `StaggerSeconds` (spreads the
  cold-start burst so Defender scanning six new processes doesn't freeze the machine).
- **Stop (today, manual):** close each bot's PowerShell window / Ctrl-C; because each window has a
  restart loop it also auto-recovers from crashes and telnet drops.
- **Status (today, manual):** infer from running `topaz.exe` processes and ESTABLISHED TCP
  connections to the server port.

### Two facts to bake into the app (discrepancies / defaults)
1. **Nodes:** `run-gembot.ps1`'s *default* `Nodes` is 25000/20000 (its header comment), but the
   `$ladder` table **explicitly overrides** to 50000/40000 (matching BOT_COMMANDS.md). The
   ladder table is the real source for Nodes → the app seeds from **50000/40000**.
2. **`run-ladder.ps1` never passes** `Time`, `AbortDepth`, `HashMb`, or `Priority` — it relies on
   `run-gembot.ps1`'s per-size **defaults**. The app must therefore either omit these args (let the
   script default) or expose them explicitly. Plan: keep them as **advanced/optional** fields with
   the script's defaults, and only pass an arg when the user overrides it.

---

## 3. Proposed architecture

```
┌─────────────────────────┐        HTTP (127.0.0.1)        ┌──────────────────────────────┐
│  React + MUI frontend   │  ───────────────────────────>  │  Node + Express backend       │
│  (Vite dev server)      │   GET/PUT config, status,      │  (bot-control server)         │
│  localhost:5173         │   start/stop/restart           │  127.0.0.1:4317               │
└─────────────────────────┘  <───────────────────────────  └───────────┬──────────────────┘
                                                                        │ child_process.spawn
                                                    reads/writes JSON    │ powershell -File run-gembot.ps1
                                                            ▼            ▼
                                              bot-control-config.json   run-gembot.ps1  ──►  topaz.exe (playtak)
                                                                        (restart loop, BelowNormal)
```

- **Frontend:** React + MUI (Vite). Talks only to the backend's REST API. No process access.
- **Backend:** Node + Express bound to `127.0.0.1`. Owns (a) the config JSON on disk and (b) the
  lifecycle of bot processes. It is the only component that touches PowerShell / the filesystem.

### The key decision: REUSE `run-gembot.ps1`, REIMPLEMENT the `run-ladder.ps1` orchestration

**Reuse `run-gembot.ps1` verbatim as the per-bot execution layer.** For each bot, the backend
spawns `powershell -File run-gembot.ps1 -User <name> -Nodes <n> ...` with args translated from the
config JSON. This keeps **one source of truth** for the proven, fiddly launch logic — the
`BelowNormal` priority drop, the `while ($true)` restart loop, the full `PLAYTAK_*` env-var mapping,
and the fixed opening/time knobs. We do **not** want to re-implement or drift from that in Node.

**Reimplement only the thin `run-ladder.ps1` orchestration** (loop over bots, stagger, `Only`
filter) in the backend. That layer is trivial (a for-loop + a delay), and reimplementing it is what
buys us the features the app needs that the script can't give: **live config from JSON** (replacing
the hand-edited `$ladder` table), **per-bot start/stop**, and **PID tracking for status**. So the
backend *is* the new `run-ladder`, but it delegates every actual launch to `run-gembot.ps1`.

Rationale for not reimplementing process mgmt in Node wholesale: the env-var contract and restart
loop are easy to get subtly wrong (locale-correct `BiasSlope` formatting, priority class, the
`UNRATED` inversion, the DBS opening flag). Reusing the script eliminates that class of bug and
means a fix to launch behavior is made in exactly one place that both the CLI and the app use.

### Process lifecycle (how the backend tracks & kills bots)

- **Start:** `child_process.spawn("powershell.exe", ["-NoProfile","-ExecutionPolicy","Bypass",
  "-File", runGembotPath, ...args])` as a **managed child** (no separate window). Capture
  `stdout`/`stderr` into a per-bot ring buffer (for the logs view and error surfacing). Store
  `{ name, pid, startedAt, child }` in an in-memory `Map` keyed by bot name. Launch order honors
  `StaggerSeconds`.
- **The grandchild problem:** `run-gembot.ps1` starts `topaz.exe` as a child of the PowerShell host.
  Killing only the PowerShell PID can orphan `topaz.exe`. **Stop** therefore uses
  `taskkill /PID <pid> /T /F` (`/T` = whole process tree) to take down the PowerShell host *and* its
  `topaz.exe` child together, defeating the restart loop.
- **Restart:** stop (await tree kill) → start.
- **Backend-lifetime coupling:** managed children die if the backend dies. For a local control
  panel that the user keeps running this is acceptable and arguably desirable (one stop button).
  The detached-window alternative (survives backend restarts) is listed as an open question (§10).

### Status polling
- **Primary source of truth:** the backend's own process map — for each managed child, is the PID
  still alive? (`process.kill(pid, 0)` / tracking `exit` events).
- **Secondary reconcile (orphan/stale detection):** on demand, enumerate `topaz.exe` processes and
  ESTABLISHED TCP connections to `Port` (via `Get-CimInstance Win32_Process` / `Get-NetTCPConnection`
  or `tasklist` + `netstat`), to detect bots running that the app didn't launch (e.g. started from
  the CLI, or a stale process after a crash). Surfaced as a "connected / running / not running /
  orphan" state per bot.
- Frontend polls `GET /api/status` every ~2–3 s (or subscribes to SSE in a later phase).

### Binding & security
- Backend binds **`127.0.0.1` only** — never `0.0.0.0`. No auth needed (local, single user).
- The bot password lives in `bot-control-config.json` in plaintext. **This is acceptable**: it's a
  throwaway bot-account password that is *already* sent in plaintext over unencrypted telnet on
  every stage. **Noted risk:** the JSON must be `.gitignore`d so a throwaway password isn't
  committed. The app will never log the password value.

---

## 4. Config schema (JSON)

Stored at `bot-control/bot-control-config.json` (git-ignored). The **6 bots are stored as 6
independent records** (that is the source of truth). "Linking" 5x5/6x6 (below) is a UI convenience
that writes to both records; it is not a separate storage form.

```jsonc
{
  "version": 1,
  "global": {
    "serverHost": "localhost",      // text
    "port": 10000,                  // number field, 1–65535
    "passwordTemplate": "password", // text; "{user}" -> each bot's username
    "rated": false,                 // switch
    "staggerSeconds": 8,            // slider 0–30 step 1
    "linkSizes": true               // UI: edit a level's 6x6 & 5x5 tunables together (default on)
  },
  "bots": [
    {
      "name": "AdvancedBot6",       // fixed identity, not editable
      "size": 6,                    // fixed, not editable
      "level": "Advanced",          // fixed, for grouping/link
      "enabled": true,              // include in "start all" / Only subset
      "tunables": {
        "nodes": 50000,             // slider 1000–200000 step 1000
        "gapMin": 20,               // slider 0–300 step 5   } invariant:
        "gapStd": 30,               // slider 0–300 step 5   } gapMin <= gapStd <= gapMax
        "gapMax": 40,               // slider 0–300 step 5   }
        "biasSlope": 8,             // slider 0–20 step 0.5 (decimal)
        "movePool": 16              // slider 1–16 step 1
      },
      "advanced": {                 // optional; omitted args fall back to run-gembot.ps1 defaults
        "abortDepth": null,         // slider 1–6 step 1 (default 1)
        "time": null,               // slider 60–1200 step 30 (default 5x5=420, 6x6=600)
        "hashMb": null,             // slider 16–512 step 16 (default 48)
        "priority": null            // select: Idle|BelowNormal|Normal|AboveNormal|High (default BelowNormal)
      }
    }
    // ... 5 more, same shape
  ]
}
```

### Slider ranges (min / max / step) and rationale

| Field | Type | Min | Max | Step | Default | Notes |
|---|---|---|---|---|---|---|
| `nodes` | slider | 1000 | 200000 | 1000 | 50000 / 40000 | current caps 50k/40k; headroom for full-strength |
| `gapMin` | slider | 0 | 300 | 5 | per level | 0 = full strength |
| `gapStd` | slider | 0 | 300 | 5 | per level | invariant middle |
| `gapMax` | slider | 0 | 300 | 5 | per level | higher = weaker |
| `biasSlope` | slider | 0 | 20 | 0.5 | 8 | decimal; worst:best odds = 1 + slope |
| `movePool` | slider | 1 | 16 | 1 | 16 | MultiPV width, engine max 16 |
| `staggerSeconds` | slider | 0 | 30 | 1 | 8 | 0 = all-at-once |
| `port` | number | 1 | 65535 | 1 | 10000 | text/number field, not a slider |
| `abortDepth` (adv) | slider | 1 | 6 | 1 | 1 | |
| `time` (adv) | slider | 60 | 1200 | 30 | 420/600 | seek base time (s) |
| `hashMb` (adv) | slider | 16 | 512 | 16 | 48 | TT size; RAM budget |
| `priority` (adv) | select | — | — | — | BelowNormal | Idle…High |

### Per-bot vs shared, and the 5x5/6x6 link
- **Per-bot (stored per record):** all `tunables` and `advanced`. Editable independently.
- **Shared / global:** `serverHost, port, passwordTemplate, rated, staggerSeconds`.
- **`linkSizes` (UI convenience, default ON):** when on, editing a level's tunables (e.g.
  "Advanced") writes the same values to **both** its 6x6 and 5x5 records — mirroring how the table
  is maintained today (identical triples/slope/pool across sizes). Turn it off to diverge a size.
  `nodes` is the natural exception (50k vs 40k differ by size); the link excludes `nodes` from the
  mirror by default, or mirrors a per-size value — decided as open question §10 (default: link
  mirrors everything **except** `nodes`).

---

## 5. Backend API design

All under `/api`, JSON, `127.0.0.1` only. Config invariants validated **server-side** too (never
trust just the UI).

### Config
- `GET  /api/config` → full config JSON.
- `PUT  /api/config` → replace full config (validates every bot's `gapMin<=gapStd<=gapMax`, ranges).
  Returns 400 with per-field errors on violation; writes to disk atomically (temp file + rename) on
  success.
- `PATCH /api/config/bots/:name` → partial update of one bot's `tunables`/`advanced`. Same
  validation. (Convenience for single-slider saves; `PUT` is the bulk path.)
- `PATCH /api/config/global` → partial update of global settings.

### Lifecycle
- `POST /api/bots/start` → start all `enabled` bots (or `{ "only": ["AdvancedBot6", ...] }`),
  staggered by `staggerSeconds`. Skips bots already running (see §9). Returns per-bot launch result.
- `POST /api/bots/stop` → stop all managed bots (`taskkill /T` each). Returns per-bot result.
- `POST /api/bots/restart` → stop-all then start-all.
- `POST /api/bots/:name/start` | `/stop` | `/restart` → single bot.
- Requests use the **currently persisted** config; launching does not implicitly save unsaved UI
  edits (the UI must save first, or the app disables Start while there are unsaved changes — §6).

### Status & logs
- `GET  /api/status` → array of `{ name, state, pid, startedAt, uptimeSec }` where `state ∈
  { running, connected, notRunning, orphan, error }`. `connected` requires the secondary TCP
  reconcile; without it, `running` (managed PID alive) is reported.
- `GET  /api/bots/:name/logs?tail=200` → recent captured stdout/stderr lines for that bot.
- `GET  /api/events` (SSE, later phase) → pushes status changes and new log lines to the UI.
- `GET  /api/health` → `{ ok: true }` for the frontend to detect the backend is up.

### Arg translation (config JSON → `run-gembot.ps1` args), server-side
For a bot record + global config:
```
-User <name> -Pass <passwordTemplate with {user}->name> -Size <size>
-Nodes <nodes> -EvalGapMin <gapMin> -EvalGapStd <gapStd> -EvalGapMax <gapMax>
-BiasSlope <biasSlope, invariant '.'> -MovePool <movePool>
-ServerHost <serverHost> -Port <port> [ -Rated ]
[ -AbortDepth <..> ] [ -Time <..> ] [ -HashMb <..> ] [ -Priority <..> ]   // only if overridden
```
Advanced fields are appended only when non-null (otherwise the script default applies). All values
are passed as separate `spawn` argv entries (no string interpolation → no shell-quoting/injection
surface).

---

## 6. Frontend layout

### Structure
- **App shell:** MUI `AppBar` (title, backend-health dot) + a top **control bar**
  (Start All / Stop All / Restart All buttons, global Rated switch, "unsaved changes" indicator,
  Save/Revert).
- **Global settings panel** (collapsible `Accordion` or a compact card): ServerHost (text), Port
  (number), Password template (text, with a helper showing the resolved example
  `AdvancedBot6…`), Rated (`Switch`), StaggerSeconds (`Slider`), Link 5x5/6x6 (`Switch`).
- **Bot grid:** grouped **by level** (Advanced / Intermediate / Novice), each level a row/section
  containing its **6x6** and **5x5** `BotCard`s side by side. Grouping by level makes the link
  toggle and the "these two should usually match" relationship visually obvious.

### `BotCard` (one per bot)
- Header: bot name, Size chip, a **status dot** (green=connected/running, grey=stopped,
  amber=orphan/connecting, red=error) + uptime, and per-bot **Start/Stop/Restart** icon buttons,
  and an `enabled` toggle.
- Sliders (MUI `Slider` with a numeric `TextField` echo for precise entry):
  - **Nodes**
  - **Gap triple** — a single grouped control (see invariant below) for GapMin/Std/Max, annotated
    "Min = losing (≤−50cp, strongest) · Std = even (0cp) · Max = winning (≥+50cp, weakest)".
  - **BiasSlope** — with live helper "worst:best odds = 1 + slope = `1+X`; 0 = uniform".
  - **MovePool**
- An **Advanced** `Accordion` (collapsed by default): AbortDepth, Time, HashMb, Priority — each
  showing "(default N)" and only sent when changed.

### Enforcing `GapMin <= GapStd <= GapMax`
Use MUI's **range-style `Slider` with three thumbs** on one track (a single `Slider` accepts an
array value `[min, std, max]`), which **inherently keeps the thumbs ordered** — MUI won't let a
lower thumb pass a higher one. This makes the invariant impossible to violate by dragging. A tiny
minimum separation (e.g. thumbs can be equal, matching `<=`) is allowed. Backup: on any manual
`TextField` entry, clamp to keep `min<=std<=max` before writing to state, and the server re-validates
on save. Show an inline error only if a value is somehow out of order (defense in depth).

### Status & unsaved-changes UX
- Status dots update from `GET /api/status` polling (2–3 s) or SSE.
- Local edits mark the config "dirty"; **Start/Restart is disabled (or prompts) while dirty**, so a
  launch always uses saved config. Save (`PUT /api/config`) clears dirty; Revert re-fetches.
- Toasts/snackbars for launch/stop results and validation/RAM warnings.

---

## 7. Tech stack & running locally (Windows)

- **Backend:** Node.js (LTS) + Express. Language: TypeScript (typed config schema is worth it) —
  open to plain JS if you'd rather keep it dead simple (§10). `child_process.spawn` for PowerShell.
  Config persisted via atomic write. Bound to `127.0.0.1`.
- **Frontend:** React 18 + Vite + MUI (`@mui/material`, `@emotion/*`), `@mui/icons-material`. Vite
  dev server proxies `/api` → backend, so the browser only ever sees one origin.
- **Layout on disk (new `bot-control/` folder in the repo):**
  ```
  bot-control/
    server/   package.json, src/ (Express, config, process manager, ps-args)
    web/      package.json, src/ (React + MUI)
    bot-control-config.json   (git-ignored; seeded from current $ladder values)
  ```
- **Ports:** backend `127.0.0.1:4317`, Vite `127.0.0.1:5173` (both avoid the telnet ports
  10000/10002). Configurable.
- **Run it:**
  ```powershell
  # one-time
  cd bot-control\server; npm install
  cd ..\web;            npm install
  # dev (two terminals, or a root concurrently script)
  cd bot-control\server; npm run dev      # Express on 4317
  cd bot-control\web;    npm run dev       # Vite on 5173 -> open the printed URL
  ```
  A convenience `run-botpanel.ps1` (opens both) can be added in a later phase.
- **`.gitignore` additions:** `bot-control/**/node_modules/`, `bot-control/**/dist/`,
  `bot-control/bot-control-config.json`.

---

## 8. Phased implementation plan (each phase independently verifiable)

- **Phase 0 — Scaffold & boot.** Create `bot-control/server` (Express, `/api/health`) and
  `bot-control/web` (Vite + MUI hello world with the AppBar). Vite proxy to backend. `.gitignore`
  updates. **Verify:** both dev servers start; the web page shows a green backend-health dot.

- **Phase 1 — Config model + persistence (read/write, no UI edit yet).** Define the schema,
  seed `bot-control-config.json` from the **current `$ladder` values** (table in §2/§4). Implement
  `GET/PUT/PATCH /api/config` with server-side validation (ranges + gap invariant) and atomic
  write. **Verify:** `curl` GET returns the seeded config; a bad PUT (gapStd<gapMin) is rejected 400;
  a good PUT round-trips to disk.

- **Phase 2 — Read-only config UI.** BotCards + global panel render the fetched config with sliders
  in a **disabled** state, grouped by level/size. **Verify:** the six bots render with the correct
  seeded values; layout matches §6.

- **Phase 3 — Editable config + invariants + save.** Enable sliders, the 3-thumb gap slider, the
  advanced accordion, dirty tracking, Save/Revert (`PUT`). **Verify:** editing + Save persists to
  JSON; the gap thumbs cannot cross; reload shows saved values; link-sizes mirrors a level across
  both sizes.

- **Phase 4 — Process management (start/stop all).** Implement the process manager: spawn
  `run-gembot.ps1` per bot with translated args, PID map, log ring buffer, `taskkill /T` stop, and
  the staggered `POST /api/bots/start` + `POST /api/bots/stop`. **Verify:** Start All launches the
  enabled bots against a local server (or a dummy host to observe the connect attempts); `topaz.exe`
  processes appear; Stop All removes them (no orphaned `topaz.exe`); relaunch works.

- **Phase 5 — Status + per-bot control.** `GET /api/status` (managed-PID state), frontend polling +
  status dots + uptime, and per-bot Start/Stop/Restart + Restart All. **Verify:** dots track reality
  as bots go up/down; killing a bot's tree externally flips its dot; per-bot restart works.

- **Phase 6 — Polish.** Secondary reconcile (orphan/`topaz.exe`/TCP-connected detection), logs
  drawer per bot, RAM-budget warning, error toasts, and optionally SSE (`/api/events`) to replace
  polling, plus the `run-botpanel.ps1` launcher. **Verify:** an externally-started bot shows as
  `orphan`; logs stream; a high total HashMb×bots shows a warning.

(Phases 4–5 are the core value; 0–3 build to them; 6 is quality-of-life. Each phase is a natural
stopping/approval point.)

---

## 9. Risks & edge cases

- **Bot already running (double-launch).** Start checks the PID map (and optionally the reconcile
  scan) and **skips** a bot that's already managed-running; returns "already running" rather than
  spawning a duplicate that would fight over the same account login.
- **Server unreachable.** `run-gembot.ps1`/`topaz.exe` retry in-process; status shows
  `running`/`connecting` but not `connected`. Surface "process up, not connected" so an unreachable
  host or wrong port is diagnosable from the panel.
- **Partial launches.** Staggered start means some bots may be up while a later one fails to spawn;
  the per-bot start result array reports each, and status reflects the partial state (no all-or-
  nothing assumption).
- **Stale PIDs / PID reuse.** Windows can reuse a PID after a process exits. Before `taskkill`,
  verify the tracked PID still maps to our process (image name `powershell`/`topaz` and, where
  cheap, the `run-gembot.ps1` command line or the recorded `startedAt`). Drop dead PIDs from the map
  on the child `exit` event so status doesn't report a ghost.
- **RAM limits.** 6 bots × (HashMb + search working set) on an 8 GB machine. The app warns if
  `sum(hashMb over enabled bots)` crosses a threshold, and keeps the default `BelowNormal` priority
  (desktop stays responsive). Advanced HashMb slider capped at 512.
- **Backend dies → managed children die.** Documented trade-off (§3). Mitigation: keep the backend
  running; open question §10 offers a detached-window mode that survives backend restarts.
- **PowerShell execution policy.** Spawn with `-NoProfile -ExecutionPolicy Bypass -File` so a
  restrictive machine policy doesn't block `run-gembot.ps1`.
- **`topaz.exe` missing / not built.** `run-gembot.ps1` already errors if
  `target\release\topaz.exe` is absent; the backend surfaces that stderr as a per-bot error toast.
- **Password in plaintext JSON.** Accepted (throwaway, already plaintext over telnet) but the file
  is git-ignored and the value is never logged. **Note the exposure.**
- **Concurrent edits / launch during edit.** "Dirty" state disables Start/Restart until saved, so a
  launch never uses half-edited config. Single-user assumption means no multi-tab locking is needed.
- **Port conflicts.** Backend `4317` / Vite `5173` chosen away from telnet `10000/10002`; both
  configurable if occupied.
- **Locale / decimal formatting.** `biasSlope` must serialize with a `.` decimal separator for the
  Rust `f64` parse (the script already does this via InvariantCulture); the backend passes the raw
  numeric string, sidestepping locale entirely.

---

## 10. Decisions & open questions

### Decided (2026-07-07)
1. **Process model → managed children.** Backend spawns each `run-gembot.ps1` as a tracked child
   (no window); one-button Stop All; bots die if the backend stops (accepted trade-off).
2. **Language → TypeScript** backend, with a config-schema type shared with the frontend.
3. **Advanced fields → shown, collapsed.** Per-BotCard Advanced accordion for AbortDepth/Time/
   HashMb/Priority, each showing its script default and only sent when changed.
4. **5x5/6x6 link → ON by default, mirrors everything except `nodes`** (50k vs 40k stay per-size).

### Still open (lower-stakes; will use the recommended default unless you say otherwise)
5. **Config location:** in-repo `bot-control/bot-control-config.json` (git-ignored, recommended) or
   under `%APPDATA%`?
6. **Stop semantics:** should Stop All also kill **orphan** `topaz.exe` the app didn't launch (full
   cleanup) or only bots it manages? *Recommended: manage-only by default, with an explicit
   "kill orphans" action.*
7. **Auto-start the backend on login** (a small scheduled task / startup script) — in scope now or
   later? *Recommended: later.*
8. **"connected" fidelity:** is managed-PID-alive enough for the status dot, or wire the TCP
   reconcile (ESTABLISHED to the server port) in from Phase 5 rather than Phase 6? *Recommended:
   PID-alive for Phase 5, TCP reconcile in Phase 6.*
```
