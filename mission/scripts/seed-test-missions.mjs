#!/usr/bin/env node
/**
 * Keeps one open copy of each ORE-CHAIN TEST MISSION on the board (local dev): the three loops of the
 * ore chain, small enough to test in minutes, always there whoever took the last copy.
 *
 *   mining           mine, crush, shelve the crushed crate in the mining village's warehouse
 *   transport        load a crushed crate there, refine it at the factory city, shelve the ingots
 *   mining-transport mine, crush, carry to the factory, refine, shelve the ingots there
 *
 * Every mission is measured on goods put INTO a warehouse (`via: warehouse`, goods moves reported by
 * the game server) and turned in at the town hall of the village it delivers to (`autoComplete: false`).
 * The warehouses are inventory POIs whose `metadata.persistence` is the cargo depot's uuid: created
 * once (their ids are kept in the state file), on the depots named in the config.
 *
 * Usage:
 *   node scripts/seed-test-missions.mjs <config.json> [--watch <seconds> | --renew]
 *   --renew cancels the open copies first, then seeds afresh (after the texts or units changed).
 *
 * config.json: { "scene": "tarsis_3", "mineral": "gold",
 *   "mine":    { "name": "M0001", "depot": "<uuid>", "x": 0, "y": 0, "z": 0 },
 *   "factory": { "name": "F0001", "depot": "<uuid>", "x": 0, "y": 0, "z": 0 } }
 * Positions: the depot's, in the planet frame (as the world DB stores it).
 *
 * Env: KEYCLOAK_URL (http://auth.dyingstar.local), KEYCLOAK_REALM (dyingstar), CLIENT_ID / CLIENT_SECRET
 * (a service account holding mission:write and inventory:poi:manage, e.g. svc-game),
 * MISSION_URL (http://services.dyingstar.local/mission), INVENTORY_URL (…/inventory).
 */
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { randomUUID } from 'node:crypto';

const [configPath, flag, every] = process.argv.slice(2);
if (!configPath) {
  console.error('usage: node scripts/seed-test-missions.mjs <config.json> [--watch <seconds>]');
  process.exit(2);
}
const config = JSON.parse(readFileSync(configPath, 'utf8'));
const statePath = configPath.replace(/\.json$/, '') + '.state.json';
const state = existsSync(statePath) ? JSON.parse(readFileSync(statePath, 'utf8')) : {};

const env = (k, d) => process.env[k] ?? d;
const KC = `${env('KEYCLOAK_URL', 'http://auth.dyingstar.local')}/realms/${env('KEYCLOAK_REALM', 'dyingstar')}`;
const MISSION = env('MISSION_URL', 'http://services.dyingstar.local/mission');
const INVENTORY = env('INVENTORY_URL', 'http://services.dyingstar.local/inventory');
/** Owner of the test POIs: a fixed id, so they are easy to find and remove. */
const TEST_OWNER = state.owner ?? (state.owner = randomUUID());

async function token() {
  const res = await fetch(`${KC}/protocol/openid-connect/token`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'client_credentials',
      client_id: env('CLIENT_ID', ''),
      client_secret: env('CLIENT_SECRET', ''),
    }),
  });
  if (!res.ok) throw new Error(`token: ${res.status} ${await res.text()}`);
  return (await res.json()).access_token;
}

async function call(base, method, path, body) {
  const res = await fetch(`${base}/api/internal${path}`, {
    method,
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${await token()}` },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

/** The warehouse POI of [site] (created once). */
async function warehousePoi(site) {
  const key = `poi:${site.depot}`;
  if (state[key]) return state[key];
  const poi = await call(INVENTORY, 'POST', '/pois', {
    owner: { type: 'mission', id: TEST_OWNER },
    name: `${site.name} — entrepôt`,
    scene: config.scene,
    x: site.x,
    y: site.y,
    z: site.z,
    radiusM: 40,
    metadata: { persistence: site.depot, test: true },
  });
  state[key] = poi.id ?? poi.poi?.id;
  console.log(`[seed] warehouse POI ${site.name}: ${state[key]}`);
  return state[key];
}

function specs(minePoi, factoryPoi) {
  const m = config.mineral ?? 'gold';
  const ore = `ore.${m}`;
  const ingot = `ingot.${m}`;
  const zone = (poiId) => ({ kind: 'poi', poiId });
  const mine = config.mine.name;
  const factory = config.factory.name;
  return {
    mining: {
      title: `[TEST] Minerai pour ${mine}`,
      titleI18n: { fr: `[TEST] Minerai pour ${mine}`, en: `[TEST] Ore for ${mine}` },
      category: 'mining',
      giver: { faction: 'ares', division: 'mining' },
      rewards: [{ type: 'credits', amount: 500 }],
      objectives: [{
        type: 'deliver_material',
        title: `Apportez une caisse de minerai concassé au comptoir de l'entrepôt de ${mine}`,
        titleI18n: {
          fr: `Apportez une caisse de minerai concassé au comptoir de l'entrepôt de ${mine}`,
          en: `Bring a crate of crushed ore to the counter of ${mine}'s warehouse`,
        },
        targetQuantity: 1,
        unit: 'kg',
        params: { goodType: ore, zone: zone(minePoi), via: 'warehouse' },
      }],
    },
    transport: {
      title: `[TEST] Raffiner le minerai de ${mine} à ${factory}`,
      titleI18n: { fr: `[TEST] Raffiner le minerai de ${mine} à ${factory}`, en: `[TEST] Refine ${mine}'s ore at ${factory}` },
      category: 'transport',
      giver: { faction: 'ares', division: 'hauling' },
      rewards: [{ type: 'credits', amount: 1500 }],
      objectives: [{
        type: 'transport',
        title: `Prenez une caisse à l'entrepôt de ${mine}, raffinez-la à ${factory}, apportez les lingots au comptoir de son entrepôt`,
        titleI18n: {
          fr: `Prenez une caisse à l'entrepôt de ${mine}, raffinez-la à ${factory}, apportez les lingots au comptoir de son entrepôt`,
          en: `Take a crate at ${mine}'s warehouse, refine it at ${factory}, bring the ingots to its warehouse counter`,
        },
        targetQuantity: 1,
        unit: 'kg',
        params: {
          goodType: ore,
          deliverGoodType: ingot,
          source: zone(minePoi),
          zone: zone(factoryPoi),
          via: 'warehouse',
        },
      }],
    },
    'mining-transport': {
      title: `[TEST] Minage et livraison de lingots à ${factory}`,
      titleI18n: { fr: `[TEST] Minage et livraison de lingots à ${factory}`, en: `[TEST] Mine and deliver ingots to ${factory}` },
      category: 'mining',
      giver: { faction: 'ares', division: 'mining' },
      rewards: [{ type: 'credits', amount: 2500 }],
      objectives: [{
        type: 'deliver_material',
        title: `Minez, concassez dans un village minier, raffinez à ${factory}, apportez les lingots au comptoir de son entrepôt`,
        titleI18n: {
          fr: `Minez, concassez dans un village minier, raffinez à ${factory}, apportez les lingots au comptoir de son entrepôt`,
          en: `Mine, crush in any mining village, refine at ${factory}, bring the ingots to its warehouse counter`,
        },
        targetQuantity: 1,
        unit: 'kg',
        params: { goodType: ingot, zone: zone(factoryPoi), via: 'warehouse' },
      }],
    },
  };
}

async function seedOnce() {
  const minePoi = await warehousePoi(config.mine);
  const factoryPoi = await warehousePoi(config.factory);
  writeFileSync(statePath, JSON.stringify(state, null, 2));
  let open = await call(MISSION, 'GET', '/missions?status=available&issuerType=system&limit=100');
  if (flag === '--renew') {
    for (const m of open.missions ?? []) {
      if (!String(m.scriptId ?? '').startsWith('test:')) continue;
      await call(MISSION, 'POST', `/missions/${m.id}/cancel`);
      console.log(`[seed] cancelled ${m.id} (${m.title})`);
    }
    open = { missions: [] };
  }
  for (const [name, spec] of Object.entries(specs(minePoi, factoryPoi))) {
    const prefix = `test:${name}:`;
    if ((open.missions ?? []).some((m) => String(m.scriptId ?? '').startsWith(prefix))) continue;
    const created = await call(MISSION, 'POST', '/missions', {
      ...spec,
      kind: 'dynamic',
      issuerType: 'system',
      autoComplete: false,
      maxAssignees: 1,
      scriptId: `${prefix}${Date.now()}`,
    });
    console.log(`[seed] ${name}: ${created.mission?.id}`);
  }
}

await seedOnce();
if (flag === '--watch') {
  const seconds = Number(every ?? 30);
  setInterval(() => seedOnce().catch((e) => console.error(`[seed] ${e.message}`)), seconds * 1000);
}
