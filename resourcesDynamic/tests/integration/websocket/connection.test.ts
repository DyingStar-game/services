import { createStandaloneWebSocket } from "@websocket/server";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { Server } from "ws";

import { TestWebSocketClient } from "../../helpers/websocket-client";

describe("WebSocket Connection", () => {
  let wss: Server;
  let client: TestWebSocketClient;

  beforeAll(() => {
    wss = createStandaloneWebSocket(3099);
  });

  afterAll(() => {
    wss.close();
  });

  it("should connect to WebSocket server", async () => {
    client = new TestWebSocketClient(3099);
    await client.connect();

    expect(client.isConnected()).toBe(true);

    // Must be receive welcome message
    const welcomeMsg = await client.waitForConnected();
    expect(welcomeMsg.type).toBe("connected");
    expect(welcomeMsg.clientId).toBeDefined();

    client.close();
  });

  it("should handle multiple concurrent connections", async () => {
    const clients = [
      new TestWebSocketClient(3099),
      new TestWebSocketClient(3099),
      new TestWebSocketClient(3099),
    ];

    await Promise.all(clients.map((c) => c.connect()));

    clients.forEach((c) => {
      expect(c.isConnected()).toBe(true);
    });

    clients.forEach((c) => c.close());
  });

  it("should disconnect cleanly", async () => {
    client = new TestWebSocketClient(3099);
    await client.connect();

    expect(client.isConnected()).toBe(true);

    client.close();

    expect(client.isConnected()).toBe(false);
  });
});
