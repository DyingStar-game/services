import { aMoon, aPlanet, aStar, aSystem } from "@builder/builders";
import { RequestInitWsType } from "@websocket/schema/Request/init.ws.model";
import { createStandaloneWebSocket } from "@websocket/server";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { Server } from "ws";

import { TestWebSocketClient } from "../../helpers/websocket-client";
import { mockSystemsWithDetails } from "../../setup";

describe("WebSocket Init", () => {
  let wss: Server;
  let client: TestWebSocketClient;

  beforeAll(() => {
    wss = createStandaloneWebSocket(3101);
  });

  afterAll(() => {
    wss.close();
  });

  beforeEach(async () => {
    client = new TestWebSocketClient(3101);
    await client.connect();
    await client.waitForConnected();
    client.clearMessages();
    mockSystemsWithDetails.length = 0;
    vi.resetModules(); // réinitialise le cache des imports
    vi.clearAllMocks(); // réinitialise les mocks
  });

  afterEach(() => {
    client.close();
  });

  it("should accept init message", async () => {
    const mockSystem = aSystem()
      .withPlanets([
        aPlanet().withMoons([aMoon().build()]).build(),
        aPlanet().withoutMoons().build(),
      ])
      .withStar(aStar().build())
      .build();

    mockSystemsWithDetails.push(mockSystem);

    const requestInit: RequestInitWsType = {
      event_type: "init",
      data: {
        duration_s: 3,
        frequency: 60,
        from_timestamp: 0,
        system_internal_name: mockSystem.name,
      },
    };

    client.send(requestInit);

    const response = await client.waitForInit();

    const systems = response.data.filter(
      (objectData) => objectData.object_type === "system",
    );

    const stars = response.data.filter(
      (objectData) => objectData.object_type === "star",
    );

    const planets = response.data.filter(
      (objectData) => objectData.object_type === "planet",
    );

    const moons = response.data.filter(
      (objectData) => objectData.object_type === "moon",
    );

    expect(response.data).toHaveLength(5); // 1 system + 1 star + 2 planets + 1 moon
    expect(moons).toHaveLength(1);
    expect(planets).toHaveLength(2);
    expect(stars).toHaveLength(1);
    expect(systems).toHaveLength(1);

    systems.forEach((system) => {
      expect(system.object_type).toBe("system");
      expect(system.object_uuid).toBe(mockSystem.uuid);
      expect(system.object_data.name).toBe(mockSystem.name);
      expect(system.object_data.scenename).toBe("");
      expect(system.object_data.from_timestamp).toBe(0);
      expect(system.object_data.parent_id).toBe("");
      expect(system.object_data.soi).toBeUndefined();
      expect(system.object_data.positions).toBeUndefined();
      expect(system.object_data.rotations).toBeUndefined();
    });

    stars.forEach((star) => {
      const mockStar = mockSystem.stars[0];
      expect(star.object_type).toBe("star");
      expect(star.object_uuid).toBe(mockStar.uuid);
      expect(star.object_data.name).toBe(mockStar.name);
      expect(star.object_data.scenename).toBe(
        `scenes/star/${mockStar.internalName}.tscn`,
      );
      expect(star.object_data.from_timestamp).toBe(0);
      expect(star.object_data.parent_id).toBe(mockSystem.uuid);
      expect(star.object_data.soi).toBeUndefined();
      expect(star.object_data.positions).toBeUndefined();
      expect(star.object_data.rotations).toBeUndefined();
    });

    planets.forEach((planet, planetIndex) => {
      const mockPlanet = mockSystem.planets[planetIndex];
      expect(planet.object_type).toBe("planet");
      expect(planet.object_uuid).toBe(mockPlanet.uuid);
      expect(planet.object_data.name).toBe(mockPlanet.name);
      expect(planet.object_data.scenename).toBe(
        `scenes/planet/${mockPlanet.internalName}.tscn`,
      );
      expect(planet.object_data.from_timestamp).toBe(0);
      expect(planet.object_data.soi).toBe(mockPlanet.radiusGravityInfluenceM);
      expect(planet.object_data.positions).toHaveLength(180); // Frequency 60 * Duration 3 = 180 positions
      expect(planet.object_data.rotations).toHaveLength(180); // Frequency 60 * Duration 3 = 180 rotations
      expect(planet.object_data.parent_id).toBe(mockSystem.uuid);
    });

    moons.forEach((moon, index) => {
      const mockPlanet = mockSystem.planets[0];
      const mockMoon = mockPlanet.moons[index];
      expect(moon.object_type).toBe("moon");
      expect(moon.object_uuid).toBe(mockMoon.uuid);
      expect(moon.object_data.name).toBe(mockMoon.name);
      expect(moon.object_data.scenename).toBe(
        `scenes/moon/${mockMoon.internalName}.tscn`,
      );
      expect(moon.object_data.from_timestamp).toBe(0);
      expect(moon.object_data.soi).toBe(mockMoon.radiusGravityInfluenceM);
      expect(moon.object_data.positions).toHaveLength(180); // Frequency 60 * Duration 3 = 180 positions
      expect(moon.object_data.rotations).toHaveLength(180); // Frequency 60 * Duration 3 = 180 rotations
      expect(moon.object_data.parent_id).toBe(mockPlanet.uuid);
    });
  });
});
