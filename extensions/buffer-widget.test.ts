import { describe, it, expect, vi, beforeEach } from "vitest";

// ── Types matching the Rust BufferSnapshot ──

interface BufferSnapshot {
  slots: boolean[];
  buffered: number;
  pending: number;
  complete: boolean;
}

// ── Pure render function extracted for testing ──

function renderBarPlain(snap: BufferSnapshot): string {
  // Mirrors the production renderBar logic — same clear conditions.
  if (snap.complete || (snap.slots.length === 0 && snap.pending === 0)) return "";
  let bar = "▐";
  for (const ready of snap.slots) {
    bar += ready ? "█" : "░";
  }
  bar += "▌";
  const label = snap.pending > 0
    ? ` ${snap.buffered} loaded, ${snap.pending} waiting`
    : ` ${snap.buffered} loaded`;
  return bar + label;
}

// ── Mock WS server ──

class MockWSServer {
  clients: MockWSClient[] = [];
  onConnection?: (client: MockWSClient) => void;

  accept(client: MockWSClient): void {
    this.clients.push(client);
    this.onConnection?.(client);
  }

  broadcast(msg: object): void {
    const data = JSON.stringify(msg);
    for (const c of this.clients) {
      c.onmessage?.({ data } as any);
    }
  }
}

class MockWSClient {
  onopen?: () => void;
  onmessage?: (event: { data: string }) => void;
  onerror?: () => void;
  onclose?: () => void;
  sent: string[] = [];

  constructor(private server: MockWSServer) {
    setTimeout(() => {
      server.accept(this);
      this.onopen?.();
    }, 0);
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.onclose?.();
  }
}

// ── Tests ──

describe("renderBarPlain", () => {
  it("complete clears", () => {
    expect(renderBarPlain({ slots: [], buffered: 0, pending: 0, complete: true })).toBe("");
  });

  it("empty not complete and not pending clears", () => {
    expect(renderBarPlain({ slots: [], buffered: 0, pending: 0, complete: false })).toBe("");
  });

  it("all loaded", () => {
    const bar = renderBarPlain({ slots: [true, true, true], buffered: 3, pending: 0, complete: false });
    expect(bar).toBe("▐███▌ 3 loaded");
  });

  it("mixed loaded and waiting", () => {
    const bar = renderBarPlain({ slots: [true, true, false, true, false], buffered: 2, pending: 3, complete: false });
    expect(bar).toBe("▐██░█░▌ 2 loaded, 3 waiting");
  });

  it("all waiting", () => {
    const bar = renderBarPlain({ slots: [false, false, false], buffered: 0, pending: 3, complete: false });
    expect(bar).toBe("▐░░░▌ 0 loaded, 3 waiting");
  });

  it("single loaded", () => {
    const bar = renderBarPlain({ slots: [true], buffered: 1, pending: 0, complete: false });
    expect(bar).toBe("▐█▌ 1 loaded");
  });

  it("drains as played", () => {
    // Simulate: 5 slots → play 2 → 3 remain
    const full = renderBarPlain({ slots: [true, true, true, true, true], buffered: 5, pending: 0, complete: false });
    const after = renderBarPlain({ slots: [true, true, true], buffered: 3, pending: 0, complete: false });
    expect(full.length).toBeGreaterThan(after.length);
  });
});

describe("BufferSnapshot JSON protocol", () => {
  it("parses Rust-format JSON", () => {
    const json = '{"slots":[true,false,true],"buffered":1,"pending":2,"complete":false}';
    const snap: BufferSnapshot = JSON.parse(json);
    expect(snap.slots).toEqual([true, false, true]);
    expect(snap.buffered).toBe(1);
    expect(snap.pending).toBe(2);
    expect(snap.complete).toBe(false);
  });

  it("renders from parsed JSON", () => {
    const json = '{"slots":[true,true,false],"buffered":2,"pending":1,"complete":false}';
    const snap: BufferSnapshot = JSON.parse(json);
    expect(renderBarPlain(snap)).toBe("▐██░▌ 2 loaded, 1 waiting");
  });
});

describe("WS message handling", () => {
  it("buffer_state message updates snapshot", async () => {
    const server = new MockWSServer();
    let lastSnapshot: BufferSnapshot | null = null;

    // Simulate what the extension does on WS message
    const handler = (event: { data: string }) => {
      const msg = JSON.parse(event.data);
      if (msg.type === "buffer_state") {
        lastSnapshot = msg.data as BufferSnapshot;
      }
    };

    const client = new MockWSClient(server);
    client.onmessage = handler;

    // Wait for connection
    await new Promise(r => setTimeout(r, 10));

    server.broadcast({
      type: "buffer_state",
      data: { slots: [true, false, true], buffered: 1, pending: 2, complete: false },
    });

    expect(lastSnapshot).not.toBeNull();
    expect(lastSnapshot!.slots).toEqual([true, false, true]);
    expect(renderBarPlain(lastSnapshot!)).toBe("▐█░█▌ 1 loaded, 2 waiting");
  });

  it("ignores non-buffer messages", async () => {
    const server = new MockWSServer();
    let updateCount = 0;

    const handler = (event: { data: string }) => {
      const msg = JSON.parse(event.data);
      if (msg.type === "buffer_state") updateCount++;
    };

    const client = new MockWSClient(server);
    client.onmessage = handler;
    await new Promise(r => setTimeout(r, 10));

    server.broadcast({ type: "emotion", emotion: "angry" });
    server.broadcast({ type: "delta", text: "hello" });
    server.broadcast({ type: "buffer_state", data: { slots: [true], buffered: 1, pending: 0, complete: false } });

    expect(updateCount).toBe(1);
  });

  it("handles malformed messages gracefully", async () => {
    const server = new MockWSServer();
    let lastSnapshot: BufferSnapshot | null = null;

    const handler = (event: { data: string }) => {
      try {
        const msg = JSON.parse(event.data);
        if (msg.type === "buffer_state") lastSnapshot = msg.data;
      } catch { /* ignore */ }
    };

    const client = new MockWSClient(server);
    client.onmessage = handler;
    await new Promise(r => setTimeout(r, 10));

    // Send garbage
    client.onmessage?.({ data: "not json" } as any);
    expect(lastSnapshot).toBeNull();

    // Then valid
    server.broadcast({
      type: "buffer_state",
      data: { slots: [true], buffered: 1, pending: 0, complete: false },
    });
    expect(lastSnapshot).not.toBeNull();
  });

  it("progressive updates drain the bar", async () => {
    const server = new MockWSServer();
    const bars: string[] = [];

    const handler = (event: { data: string }) => {
      const msg = JSON.parse(event.data);
      if (msg.type === "buffer_state") {
        bars.push(renderBarPlain(msg.data));
      }
    };

    const client = new MockWSClient(server);
    client.onmessage = handler;
    await new Promise(r => setTimeout(r, 10));

    // 5 chunks total, arriving and draining
    server.broadcast({ type: "buffer_state", data: { slots: [true, false, false, false, false], buffered: 1, pending: 4, complete: false } });
    server.broadcast({ type: "buffer_state", data: { slots: [true, true, false, false], buffered: 2, pending: 2, complete: false } });
    server.broadcast({ type: "buffer_state", data: { slots: [true, true], buffered: 2, pending: 0, complete: false } });
    server.broadcast({ type: "buffer_state", data: { slots: [], buffered: 0, pending: 0, complete: true } });

    expect(bars).toEqual([
      "▐█░░░░▌ 1 loaded, 4 waiting",
      "▐██░░▌ 2 loaded, 2 waiting",
      "▐██▌ 2 loaded",
      "",  // complete → clears
    ]);
  });
});
