import { aSystem } from "@builder/builders";
import { RequestInitWsType } from "@websocket/schema/Request/init.ws.model";
import { createStandaloneWebSocket } from "@websocket/server";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { Server } from "ws";

import { TestWebSocketClient } from "../../helpers/websocket-client";
import { mockSystemsWithDetails } from "../../setup";

describe("WebSocket Message Format", () => {
  let wss: Server;
  let client: TestWebSocketClient;

  beforeAll(() => {
    wss = createStandaloneWebSocket(3100);
  });

  afterAll(() => {
    wss.close();
  });

  beforeEach(async () => {
    client = new TestWebSocketClient(3100);
    await client.connect();
    await client.waitForMessage(); // Welcome message
    client.clearMessages();
  });

  afterEach(() => {
    client.close();
  });

  it("should reject invalid MessagePack", async () => {
    client["ws"]?.send(Buffer.from("invalid data"));

    const response = await client.waitForError();

    expect(response.error).toBeDefined();
  });

  it("should reject invalid JSON structure", async () => {
    client.send({ invalid: "structure" });

    const response = await client.waitForError();

    expect(response.error).toBeDefined();
  });

  it("should accept init message", async () => {
    const system = aSystem().build();
    mockSystemsWithDetails.push(system);

    const requestInit: RequestInitWsType = {
      event_type: "init",
      data: {
        duration_s: 3,
        frequency: 60,
        from_timestamp: 0,
        system_internal_name: system.name,
      },
    };

    client.send(requestInit);

    const response = await client.waitForInit();

    expect(response.event).toBe("create_object");
    expect(response.namespace).toBe("genericprops");
  });
});
