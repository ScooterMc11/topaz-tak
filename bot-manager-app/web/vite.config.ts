import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Dev server on 5173. All /api calls are proxied to the backend so the browser sees a single origin
// (no CORS). Backend stays on 127.0.0.1:4317 (loopback) — the proxy runs here on the PC, so only THIS
// server needs to be reachable from other devices (e.g. your phone on the same Wi-Fi).
//
// host: true binds all interfaces (loopback + LAN + any VPN like Tailscale), so `npm run dev` prints
// a "Network" URL like http://192.168.x.x:5173 you can open on your phone. Local access is unchanged.
// There is NO auth on this panel, so only broaden this on networks you trust (home Wi-Fi / Tailscale),
// and open the Windows Firewall for TCP 5173 on the Private profile. Set to "127.0.0.1" to lock it
// back down to this machine only.
export default defineConfig({
  plugins: [react()],
  server: {
    host: true,
    port: 5173,
    proxy: {
      "/api": {
        target: "http://127.0.0.1:4317",
        changeOrigin: true,
      },
    },
  },
});
