// Translate a bot's config + global settings into run-gembot.ps1 arguments — the same invocation
// run-ladder.ps1 builds, so the proven launch logic is the single source of truth. Values are
// passed as separate argv entries (no shell string), so there's no quoting/injection surface.
import { AppConfig, BotConfig, UNDO_OFFERS_BY_LEVEL } from "../config/schema.js";

// The UI works in a cp scale that's 1/4 of topaz's internal eval scale. Multiply the eval-gap
// bounds by this when handing them to the engine so the UI can keep its human-friendly numbers.
const GAP_SCALE = 4;

export function resolvePassword(template: string, botName: string): string {
  return template.replace("{user}", botName);
}

export function buildGembotArgs(bot: BotConfig, global: AppConfig["global"]): string[] {
  const t = bot.tunables;
  const args: string[] = [
    "-User", bot.name,
    "-Pass", resolvePassword(global.passwordTemplate, bot.name),
    "-Size", String(bot.size),
    // Strength knob: pass whichever mode is active and zero the other so it can't interfere.
    // Engine: Depth > 0 => fixed-depth (node cap ignored); Depth 0 => the node cap applies.
    ...(t.searchMode === "depth"
      ? ["-Depth", String(t.depth), "-Nodes", "0"]
      : ["-Depth", "0", "-Nodes", String(t.nodes)]),
    // The UI's cp scale is 1/4 of topaz's internal eval scale, so scale the gap bounds by 4 here.
    // (UI 30/40/50 -> engine 120/160/200.) The UI keeps showing the smaller, human-friendly numbers.
    "-EvalGapMin", String(t.gapMin * GAP_SCALE),
    "-EvalGapStd", String(t.gapStd * GAP_SCALE),
    "-EvalGapMax", String(t.gapMax * GAP_SCALE),
    // JS number->string always uses '.' as the decimal separator (matches the script's invariant
    // formatting for the Rust f64 parse).
    "-BiasSlope", String(t.biasSlope),
    // Same 1/4 UI cp scale as the gap bounds: an always-allow floor of +50cp -> engine +200.
    "-AlwaysAllow", String(t.alwaysAllow * GAP_SCALE),
    "-MovePool", String(t.movePool),
    // Courtesy undos: the per-level cap when the switch is on, 0 (= off) when it isn't.
    "-UndoOffers", String(bot.offerUndos ? UNDO_OFFERS_BY_LEVEL[bot.level] : 0),
    "-ServerHost", global.serverHost,
    "-Port", String(global.port),
  ];
  if (global.rated) args.push("-Rated");

  // Advanced overrides: only pass when set (null => run-gembot.ps1's own default applies).
  const a = bot.advanced;
  if (a.abortDepth != null) args.push("-AbortDepth", String(a.abortDepth));
  if (a.time != null) args.push("-Time", String(a.time));
  if (a.hashMb != null) args.push("-HashMb", String(a.hashMb));
  if (a.priority != null) args.push("-Priority", a.priority);

  return args;
}
