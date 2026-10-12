import assert from "node:assert/strict";
import { test } from "node:test";

import { createCache } from "../src/cache.js";

test("returns undefined on miss", () => {
  const cache = createCache();
  assert.equal(cache.get("a"), undefined);
});

test("stores and returns a value", () => {
  const cache = createCache();
  cache.set("a", 42);
  assert.equal(cache.get("a"), 42);
});

test("expires after the ttl", () => {
  let clock = 0;
  const cache = createCache({ ttlMs: 100, now: () => clock });
  cache.set("a", 1);
  clock = 99;
  assert.equal(cache.get("a"), 1);
  clock = 100;
  assert.equal(cache.get("a"), undefined);
});

test("caps the number of entries, dropping the oldest", () => {
  const cache = createCache({ max: 2 });
  cache.set("a", 1);
  cache.set("b", 2);
  cache.set("c", 3);
  assert.equal(cache.size, 2);
  assert.equal(cache.get("a"), undefined);
  assert.equal(cache.get("c"), 3);
});

test("getOrSet caches the resolved value", async () => {
  const cache = createCache();
  let calls = 0;
  const load = async () => {
    calls += 1;
    return "v";
  };
  assert.equal(await cache.getOrSet("k", load), "v");
  assert.equal(await cache.getOrSet("k", load), "v");
  assert.equal(calls, 1);
});
