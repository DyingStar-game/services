// Tiny in-memory TTL cache. One entry per (check type, players) key; entries expire
// after TTL ms and the map is capped so a flood of distinct ids cannot grow it unbound.
// Values are the resolved check results (booleans); failures are not cached.

const DEFAULT_TTL_MS = 30_000;
const DEFAULT_MAX = 10_000;

export function createCache({ ttlMs = DEFAULT_TTL_MS, max = DEFAULT_MAX, now = Date.now } = {}) {
  const store = new Map();

  function evictExpired() {
    const t = now();
    for (const [key, entry] of store) {
      if (entry.expiresAt <= t) store.delete(key);
    }
  }

  function evictOldest() {
    // Map preserves insertion order; drop the oldest live entry.
    for (const key of store.keys()) {
      store.delete(key);
      return;
    }
  }

  return {
    get(key) {
      const entry = store.get(key);
      if (!entry) return undefined;
      if (entry.expiresAt <= now()) {
        store.delete(key);
        return undefined;
      }
      return entry.value;
    },

    set(key, value) {
      if (store.has(key)) store.delete(key);
      else if (store.size >= max) {
        evictExpired();
        while (store.size >= max) evictOldest();
      }
      store.set(key, { value, expiresAt: now() + ttlMs });
    },

    /** Returns the cached value or runs `fn`, caches the result and returns it. */
    async getOrSet(key, fn) {
      const hit = this.get(key);
      if (hit !== undefined) return hit;
      const value = await fn();
      if (value !== undefined) this.set(key, value);
      return value;
    },

    get size() {
      return store.size;
    },

    clear() {
      store.clear();
    },
  };
}
