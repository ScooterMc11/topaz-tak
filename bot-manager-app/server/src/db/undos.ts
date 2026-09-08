// Counting courtesy undos out of a bot's captured log.
//
// The engine prints one line per undo the opponent actually accepted, so the log is the record —
// there's no separate signal over the telnet protocol and nothing else to cross-check against.
// Both the insert path (games.ts) and the one-time backfill (db.ts) go through here so the two can
// never drift apart.
//
// Matched against src/topaz.rs: `println!("Undo accepted: last move rolled back")`. Offers that
// lapsed or were waved off with "gg" are NOT counted — this is "how often did someone take the
// take-back", not "how often was one offered".
const UNDO_ACCEPTED_MARKER = "Undo accepted:";

export function countUndosAccepted(lines: string[]): number {
  return lines.filter((l) => l.includes(UNDO_ACCEPTED_MARKER)).length;
}
