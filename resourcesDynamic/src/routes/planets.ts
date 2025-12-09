import { createPlanetSchema, updatePlanetSchema } from "@db/schema";
import { Hono } from "hono";
import { errorHandler } from "src/middleware/error-handler";

import {
  createPlanet,
  deletePlanet,
  getAllPlanets,
  getPlanetByUuid,
  getPlanetsBySystemId,
  updatePlanet,
} from "../db/queries";

const planetsRouter = new Hono();

planetsRouter.onError(errorHandler);

/**
 * GET /planets - Lists all planets
 */
planetsRouter.get("/", async (c) => {
  const planetsList = await getAllPlanets();
  return c.json({
    success: true,
    count: planetsList.length,
    data: planetsList,
  });
});

/**
 * GET /planets/:uuid - Retrieves a planet by UUID with its moons
 */
planetsRouter.get("/:uuid", async (c) => {
  const uuid = c.req.param("uuid");
  const planet = await getPlanetByUuid(uuid);

  if (!planet) {
    return c.json({ success: false, error: "Planet not found" }, 404);
  }

  return c.json({ success: true, data: planet });
});

/**
 * GET /planets/system/:systemId - Planets in a system
 */
planetsRouter.get("/system/:systemId", async (c) => {
  const systemId = parseInt(c.req.param("systemId"));
  const planetsList = await getPlanetsBySystemId(systemId);

  return c.json({
    success: true,
    count: planetsList.length,
    data: planetsList,
  });
});

/**
 * POST /planets - Create a new planet
 */
planetsRouter.post("/", async (c) => {
  const body: unknown = await c.req.json();
  const validated = createPlanetSchema.parse(body);

  const newPlanet = await createPlanet(validated);

  return c.json({ success: true, data: newPlanet }, 201);
});

/**
 * PATCH /planets/:uuid - Updates a planet
 */
planetsRouter.patch("/:uuid", async (c) => {
  const uuid = c.req.param("uuid");
  const body: unknown = await c.req.json();
  const validated = updatePlanetSchema.parse(body);

  const updated = await updatePlanet(uuid, validated);

  if (!updated) {
    return c.json({ success: false, error: "Planet not found" }, 404);
  }

  return c.json({ success: true, data: updated });
});

/**
 * DELETE /planets/:uuid - Delete a planet
 */
planetsRouter.delete("/:uuid", async (c) => {
  const uuid = c.req.param("uuid");
  const deleted = await deletePlanet(uuid);

  if (!deleted) {
    return c.json({ success: false, error: "Planet not found" }, 404);
  }

  return c.json({ success: true, message: "Planet deleted" });
});

export default planetsRouter;
