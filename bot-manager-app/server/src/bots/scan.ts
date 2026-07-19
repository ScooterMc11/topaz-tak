// System scan for status enrichment: enumerate topaz.exe processes (+ their parent PID), the PIDs
// with an ESTABLISHED TCP connection to the server port (= a bot that's actually connected), and
// run-gembot.ps1 PowerShell hosts (to spot bots started outside this panel). One PowerShell call.
import { spawn } from "node:child_process";

export interface ScanTopaz {
  pid: number;
  ppid: number;
}
export interface ScanHost {
  pid: number;
  user: string;
}
export interface SystemScan {
  topaz: ScanTopaz[];
  conns: number[]; // owning PIDs of ESTABLISHED connections to the server port
  hosts: ScanHost[]; // run-gembot.ps1 hosts (with a -User arg)
}

const EMPTY: SystemScan = { topaz: [], conns: [], hosts: [] };

const toArray = <T>(x: T | T[] | null | undefined): T[] =>
  Array.isArray(x) ? x : x == null ? [] : [x];

function buildScript(port: number): string {
  return [
    `$p=${port}`,
    `$t=@(Get-CimInstance Win32_Process -Filter "Name='topaz.exe'" | ForEach-Object { @{pid=$_.ProcessId;ppid=$_.ParentProcessId} })`,
    `$c=@(Get-NetTCPConnection -State Established -RemotePort $p -ErrorAction SilentlyContinue | ForEach-Object { $_.OwningProcess })`,
    // Exclude our own scan process (it mentions run-gembot.ps1 + -User in its filter/regex) via its
    // Get-NetTCPConnection signature, so the scan doesn't report itself as an orphan bot.
    `$h=@(Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" | Where-Object { $_.CommandLine -like '*run-gembot.ps1*' -and $_.CommandLine -like '*-User *' -and $_.CommandLine -notlike '*Get-NetTCPConnection*' } | ForEach-Object { $u='';if($_.CommandLine -match '-User\\s+(\\S+)'){$u=$Matches[1]}; @{pid=$_.ProcessId;user=$u} })`,
    `[pscustomobject]@{topaz=$t;conns=$c;hosts=$h} | ConvertTo-Json -Depth 4 -Compress`,
  ].join("; ");
}

export function scanSystem(port: number): Promise<SystemScan> {
  return new Promise((resolve) => {
    let out = "";
    const ps = spawn(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-Command", buildScript(port)],
      { windowsHide: true }
    );
    ps.stdout?.setEncoding("utf8");
    ps.stdout?.on("data", (d: string) => (out += d));
    ps.on("error", () => resolve(EMPTY));
    ps.on("exit", () => {
      try {
        const j = JSON.parse(out.trim());
        resolve({
          topaz: toArray<ScanTopaz>(j.topaz).map((t) => ({ pid: Number(t.pid), ppid: Number(t.ppid) })),
          conns: toArray<number>(j.conns).map((n) => Number(n)),
          hosts: toArray<ScanHost>(j.hosts).map((h) => ({ pid: Number(h.pid), user: String(h.user) })),
        });
      } catch {
        resolve(EMPTY);
      }
    });
  });
}
