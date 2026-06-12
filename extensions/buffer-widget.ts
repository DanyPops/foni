/**
 * Depecher Buffer Widget — live playback buffer bar in the TUI.
 *
 * Connects to depecherd WS and renders a FIFO drain bar
 * that updates in real-time as TTS chunks arrive and play.
 *
 * ▐███·█·▌  3 ready, 2 pending
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const WS_URL = process.env.DEPECHER_WS_URL ?? "ws://localhost:5050/ws";
const RECONNECT_MS = 5_000;

interface BufferSnapshot {
  slots: boolean[];
  buffered: number;
  pending: number;
  complete: boolean;
}

function renderBar(snap: BufferSnapshot, theme: any): string {
  if (snap.complete || (snap.slots.length === 0 && snap.pending === 0)) return "";

  let bar = theme.fg("dim", "▐");
  for (const ready of snap.slots) {
    bar += ready ? theme.fg("accent", "█") : theme.fg("dim", "░");
  }
  bar += theme.fg("dim", "▌");

  const label = snap.pending > 0
    ? theme.fg("muted", ` ${snap.buffered} loaded, ${snap.pending} waiting`)
    : theme.fg("success", ` ${snap.buffered} loaded`);

  return bar + label;
}

export default function (pi: ExtensionAPI) {
  let ws: WebSocket | null = null;
  let snapshot: BufferSnapshot | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let visible = false;

  function connect(): void {
    try {
      ws = new WebSocket(WS_URL);
    } catch {
      scheduleReconnect();
      return;
    }

    ws.onopen = () => {
      // no-op: we only listen for buffer_state messages
    };

    ws.onmessage = (event) => {
      try {
        const msg = JSON.parse(String(event.data));
        if (msg.type === "buffer_state") {
          snapshot = msg.data as BufferSnapshot;
          updateWidget();
        }
      } catch { /* ignore malformed */ }
    };

    ws.onerror = () => {
      ws?.close();
    };

    ws.onclose = () => {
      ws = null;
      scheduleReconnect();
    };
  }

  function scheduleReconnect(): void {
    if (reconnectTimer) return;
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      if (!ws) connect();
    }, RECONNECT_MS);
  }

  function updateWidget(): void {
    if (!snapshot || !visible) return;
    const bar = renderBar(snapshot, {} as any);
    if (!bar) {
      pi.ui.setWidget("depecher-buffer", undefined);
      return;
    }
    pi.ui.setWidget("depecher-buffer", (_tui: any, theme: any) => ({
      render: () => [renderBar(snapshot!, theme)],
      invalidate: () => {},
    }), { placement: "belowEditor" });
  }

  function show(): void {
    visible = true;
    if (!ws) connect();
    if (snapshot) updateWidget();
  }

  function hide(): void {
    visible = false;
    pi.ui.setWidget("depecher-buffer", undefined);
  }

  // Auto-show when depecherd is active
  pi.on("before_agent_start", async () => {
    show();
    return undefined;
  });

  pi.registerCommand("depecher-buffer", {
    description: "Toggle depecher playback buffer bar",
    handler: async (_args, ctx) => {
      if (visible) {
        hide();
        ctx.ui.notify("Buffer bar hidden", "info");
      } else {
        show();
        ctx.ui.notify("Buffer bar visible", "info");
      }
    },
  });
}
