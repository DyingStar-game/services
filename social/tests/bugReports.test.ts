/**
 * In-game bug reports: relayed to the Discord webhook of the environment, under the name of the
 * player the TOKEN names, with mentions switched off; off when no webhook is configured; paced
 * per player. Discord is a stubbed `fetch`: no report leaves the test.
 */
import type { AddressInfo } from 'net';
import type { Server } from 'http';
import express from 'express';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

const WEBHOOK = 'https://discord.test/api/webhooks/1/secret-token';

const bug = vi.hoisted(() => ({
  webhookUrl: '',
  forum: false,
  forumTagIds: [] as string[],
  environment: 'preprod',
  minIntervalSeconds: 60,
  dailyMax: 20,
}));

vi.mock('../src/config/env.js', async (importOriginal) => {
  const original = await importOriginal<typeof import('../src/config/env.js')>();
  return { env: { ...original.env, bugReport: bug } };
});

const { errorHandler } = await import('../src/middleware/errorHandler.js');
const { bugReportsRoutes } = await import('../src/routes/bugReports.routes.js');
const { resetBugReportLimits } = await import('../src/services/bugReports.service.js');

const PLAYER = { id: '9a8b7c6d-5e4f-4a3b-b2c1-0d9e8f7a6b5c', username: 'The_Moye', roles: [] as string[] };
/** The smallest thing that reads as a JPEG. */
const JPEG = Buffer.from([0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10]).toString('base64');
const REPORT = {
  title: 'Stuck in the truck',
  description: 'Teleported from the driver seat, cannot move.',
  context: { version: '0.0.1-dev', server: 'ut-22878', place: 'Tarsis3' },
  screenshot: JPEG,
  log: 'last lines of godot.log',
};

let server: Server;
let baseUrl: string;
const realFetch = globalThis.fetch;
/** What reached "Discord": the URL and the multipart form of each call. */
let relayed: { url: string; form: FormData }[] = [];
let discordStatus = 200;

async function send(body: unknown): Promise<{ status: number; body: any }> {
  const res = await realFetch(`${baseUrl}/api/bug-reports`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  return { status: res.status, body: await res.json().catch(() => null) };
}

function payloadOf(call: { form: FormData }): any {
  return JSON.parse(String(call.form.get('payload_json')));
}

beforeAll(async () => {
  const app = express();
  app.use(express.json({ limit: '8mb' }));
  // The player comes from the token in production (playerAuth); a stub identity stands in here.
  app.use('/api/bug-reports', (req, _res, next) => {
    req.player = PLAYER;
    next();
  }, bugReportsRoutes);
  app.use(errorHandler);
  server = await new Promise<Server>((resolve) => {
    const listening = app.listen(0, () => resolve(listening));
  });
  baseUrl = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});

afterAll(() => {
  server.close();
});

beforeEach(() => {
  relayed = [];
  discordStatus = 200;
  Object.assign(bug, { webhookUrl: WEBHOOK, forum: false, forumTagIds: [], environment: 'preprod', minIntervalSeconds: 60, dailyMax: 20 });
  resetBugReportLimits();
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
  vi.spyOn(console, 'log').mockImplementation(() => undefined);
  vi.stubGlobal('fetch', vi.fn(async (url: string, init: RequestInit) => {
    relayed.push({ url: String(url), form: init.body as FormData });
    return new Response(discordStatus === 200 ? '{"id":"1"}' : '{"message":"Unknown Webhook"}', { status: discordStatus });
  }));
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('POST /api/bug-reports', () => {
  it('relays the report to the webhook, titled with the environment and the build', async () => {
    const { status, body } = await send(REPORT);
    expect(status).toBe(201);
    expect(body).toEqual({ delivered: true });
    expect(relayed).toHaveLength(1);
    expect(relayed[0].url).toBe(`${WEBHOOK}?wait=true`);
    const payload = payloadOf(relayed[0]);
    expect(payload.embeds[0].title).toBe('[BUG][PREPROD][0.0.1-dev] Stuck in the truck');
    expect(payload.embeds[0].description).toBe(`**Description**\n${REPORT.description}`);
    expect(payload.embeds[0].fields).toContainEqual({ name: 'server', value: 'ut-22878', inline: true });
    expect(payload.embeds[0].image).toEqual({ url: 'attachment://screenshot.jpg' });
    expect(payload.thread_name).toBeUndefined();
  });

  it('lays the report out on the QA template, in the language of the player', async () => {
    const template = { ...REPORT, steps: '1. Sit\n2. Teleport', actual: 'Frozen', expected: 'Still at the wheel' };
    await send(template);
    expect(payloadOf(relayed[0]).embeds[0].description).toBe(
      `**Description**\n${REPORT.description}\n\n**Steps**\n1. Sit\n2. Teleport\n\n**Actual result**\nFrozen\n\n**Expected result**\nStill at the wheel`,
    );
    resetBugReportLimits();
    const { runWithLang } = await import('../src/i18n/lang.js');
    const { reportBody } = await import('../src/services/bugReports.service.js');
    expect(runWithLang('fr', () => reportBody(template))).toContain('**Étapes**\n1. Sit');
    expect(runWithLang('fr', () => reportBody(template))).toContain('**Résultat attendu**\nStill at the wheel');
  });

  it('names the player of the token and lets no mention through', async () => {
    await send({ ...REPORT, title: '@everyone look', context: { ...REPORT.context, username: 'someone else' } });
    const payload = payloadOf(relayed[0]);
    expect(payload.embeds[0].footer.text).toBe(`The_Moye · ${PLAYER.id}`);
    expect(payload.allowed_mentions).toEqual({ parse: [] });
  });

  it('attaches the capture and the log', async () => {
    await send(REPORT);
    const capture = relayed[0].form.get('files[0]') as File;
    const log = relayed[0].form.get('files[1]') as File;
    expect(capture.name).toBe('screenshot.jpg');
    expect(capture.type).toBe('image/jpeg');
    expect(log.name).toBe('godot-log.txt');
    expect(await log.text()).toBe(REPORT.log);
  });

  it('opens a post named after the title when the channel is a forum', async () => {
    Object.assign(bug, { forum: true, forumTagIds: ['42'] });
    await send(REPORT);
    const payload = payloadOf(relayed[0]);
    expect(payload.thread_name).toBe('[BUG][PREPROD][0.0.1-dev] Stuck in the truck');
    expect(payload.applied_tags).toEqual(['42']);
  });

  it('is off without a webhook, and says so', async () => {
    bug.webhookUrl = '';
    const status = await realFetch(`${baseUrl}/api/bug-reports/status`).then((res) => res.json());
    expect(status).toEqual({ enabled: false });
    const { status: code, body } = await send(REPORT);
    expect(code).toBe(503);
    expect(body.error).toBe('BUG_REPORT_DISABLED');
    expect(relayed).toHaveLength(0);
  });

  it('paces a player: one report a minute', async () => {
    expect((await send(REPORT)).status).toBe(201);
    const second = await send(REPORT);
    expect(second.status).toBe(429);
    expect(second.body.error).toBe('BUG_REPORT_RATE_LIMITED');
    expect(relayed).toHaveLength(1);
  });

  it('does not count a report Discord refused, and never answers with the webhook', async () => {
    discordStatus = 404;
    const refused = await send(REPORT);
    expect(refused.status).toBe(502);
    expect(refused.body.error).toBe('BUG_REPORT_UPSTREAM');
    expect(JSON.stringify(refused.body)).not.toContain('secret-token');
    discordStatus = 200;
    expect((await send(REPORT)).status).toBe(201);
  });

  it('refuses a capture that is not an image, and a title too short', async () => {
    const notAnImage = await send({ ...REPORT, screenshot: Buffer.from('<script>').toString('base64') });
    expect(notAnImage.status).toBe(400);
    expect(notAnImage.body.error).toBe('BUG_REPORT_BAD_SCREENSHOT');
    const noTitle = await send({ ...REPORT, title: 'a' });
    expect(noTitle.status).toBe(400);
    expect(noTitle.body.error).toBe('VALIDATION_ERROR');
    expect(relayed).toHaveLength(0);
  });
});
