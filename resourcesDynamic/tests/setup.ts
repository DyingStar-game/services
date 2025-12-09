import { SystemWithDetails } from "@db/schema";
import * as dotenv from "dotenv";
import { afterAll, beforeAll, vi } from "vitest";

dotenv.config({ path: ".env.test", override: false });

export const mockSystemsWithDetails: SystemWithDetails[] = [];

vi.mock("@lib/logger", () => {
  const createMockLogger = () => ({
    silly: vi.fn(),
    trace: vi.fn(),
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
    fatal: vi.fn(),
    child: vi.fn(() => createMockLogger()),
  });

  return {
    logger: createMockLogger(),
    wsLogger: createMockLogger(),
    pgDbLogger: createMockLogger(),
    apiLogger: createMockLogger(),
    cacheLogger: createMockLogger(),
    cacheTransformLogger: createMockLogger(),
    keplerOrbitServiceLogger: createMockLogger(),
    keplerOrbitLogger: createMockLogger(),
    logError: vi.fn(),
    logPerformance: vi.fn(),
    createTimer: vi.fn(() => ({
      end: vi.fn(() => 100),
    })),
    isDev: true,
    isProd: false,
  };
});

vi.mock("@db/queries/systems.ts", () => {
  return {
    getAllSystemsWithDetails: vi.fn(() =>
      Promise.resolve(mockSystemsWithDetails),
    ),
    getSystemWithDetailsByInternalName: vi.fn(() =>
      Promise.resolve(mockSystemsWithDetails[0] ?? undefined),
    ),
  };
});

beforeAll(() => {
  console.log("🧪 Setting up tests...");
});

afterAll(() => {
  console.log("✅ Tests completed");
});
