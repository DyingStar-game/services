import { createSystemSchema, updateSystemSchema } from "@db/schema";
import { Hono } from "hono";
import { errorHandler } from "src/middleware/error-handler";

import {
  createSystem,
  getAllSystems,
  getSystemWithDetails,
  updateSystem,
} from "../db/queries";

const systemsRouter = new Hono();

systemsRouter.onError(errorHandler);

/**
 * GET /systems - List all systems
 */
systemsRouter.get("/", async (c) => {
  const systemsList = await getAllSystems();
  return c.json({
    success: true,
    count: systemsList.length,
    data: systemsList,
  });
});

/**
 * GET /systems/:id - Retrieves a system with its planets and stars
 */
systemsRouter.get("/:id", async (c) => {
  const id = parseInt(c.req.param("id"));

  const system = await getSystemWithDetails(id);

  if (!system) {
    return c.json({ success: false, error: "System not found" }, 404);
  }

  return c.json({ success: true, data: system });
});

/**
 * POST /systems - Creates a new system
 */
systemsRouter.post("/", async (c) => {
  const body: unknown = await c.req.json();
  const validated = createSystemSchema.parse(body);

  const newSystem = await createSystem(validated);

  return c.json({ success: true, data: newSystem }, 201);
});

/**
 * PATCH /systems/:id - Updates a system
 */
systemsRouter.patch("/:id", async (c) => {
  const id = parseInt(c.req.param("id"));
  const body: unknown = await c.req.json();
  const validated = updateSystemSchema.parse(body);

  const updated = await updateSystem(id, validated);

  if (!updated) {
    return c.json({ success: false, error: "System not found" }, 404);
  }

  return c.json({ success: true, data: updated });
});

export default systemsRouter;
