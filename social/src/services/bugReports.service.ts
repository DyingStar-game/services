/**
 * In-game bug reports, relayed to a Discord webhook.
 *
 * The game sends a title, a description, the context it knows (build, server, place) and the
 * capture it just took; this service — the only holder of the webhook URL, which is a secret
 * and must never ship in the open-source client — posts it to the QA channel. One webhook per
 * environment: the title carries the environment label so a shared channel stays readable.
 *
 * Nothing is stored: a report lives in Discord. An empty `DISCORD_BUG_WEBHOOK_URL` turns the
 * feature off (503), which is the default everywhere.
 */
import { env } from '../config/env.js';
import { t } from '../i18n/index.js';
import { HttpError } from '../lib/httpError.js';
import type { AuthenticatedPlayer } from '../middleware/auth.js';

/** Discord limits the service must stay under (thread name, embed title/description/field). */
const THREAD_NAME_MAX = 100;
const EMBED_TITLE_MAX = 256;
const EMBED_FIELD_VALUE_MAX = 1024;
/** Context entries shown as embed fields (Discord allows 25). */
const CONTEXT_FIELDS_MAX = 20;
/** Largest capture accepted once decoded. */
const SCREENSHOT_MAX_BYTES = 4 * 1024 * 1024;

export interface BugReportInput {
  title: string;
  description: string;
  /** How to get there, what happened, what should have: the rest of the QA template. */
  steps?: string;
  actual?: string;
  expected?: string;
  /** What the client knows of where and on what it happened; shown as-is, never trusted. */
  context?: Record<string, string | number | boolean>;
  /** The capture, base64 (JPEG or PNG). */
  screenshot?: string;
  /** The tail of the client's log. */
  log?: string;
}

/** Sending times per player (ms), pruned to the last day on each call. */
const sentAt = new Map<string, number[]>();
const DAY_MS = 24 * 60 * 60 * 1000;

/** Whether bug reports are relayed on this environment. */
export function bugReportsEnabled(): boolean {
  return env.bugReport.webhookUrl !== '';
}

/** Forgets every sending time (tests). */
export function resetBugReportLimits(): void {
  sentAt.clear();
}

/**
 * Refuses a player sending too fast or too many, and records the sending otherwise.
 * In memory: a restart or a second replica loosens it, which a QA tool can afford.
 */
function takeSlot(playerId: string, now: number): void {
  const recent = (sentAt.get(playerId) ?? []).filter((at) => now - at < DAY_MS);
  const last = recent[recent.length - 1];
  const wait = last === undefined ? 0 : env.bugReport.minIntervalSeconds * 1000 - (now - last);
  if (wait > 0 || recent.length >= env.bugReport.dailyMax) {
    throw new HttpError(429, 'BUG_REPORT_RATE_LIMITED', 'Too many bug reports: try again later');
  }
  recent.push(now);
  sentAt.set(playerId, recent);
}

/** Gives back the slot of a report that never reached Discord. */
function releaseSlot(playerId: string, now: number): void {
  const recent = sentAt.get(playerId);
  if (!recent) return;
  const index = recent.lastIndexOf(now);
  if (index >= 0) recent.splice(index, 1);
}

function clip(text: string, max: number): string {
  return text.length <= max ? text : `${text.slice(0, max - 1)}…`;
}

/** `[BUG][PREPROD][0.0.1-dev] The title`: the label of the environment, then the client's build. */
export function reportTitle(input: BugReportInput): string {
  const environment = env.bugReport.environment.toUpperCase();
  const version = input.context?.version;
  const tags = ['BUG', environment, version === undefined ? '' : String(version)]
    .filter((tag) => tag !== '')
    .map((tag) => `[${tag}]`)
    .join('');
  return clip(`${tags} ${input.title}`, THREAD_NAME_MAX);
}

/**
 * The report's text on the QA template: one titled section per part the player filled in, in the
 * language they wrote it in (the request's).
 */
export function reportBody(input: BugReportInput): string {
  const sections: [string, string | undefined][] = [
    [t('bug_report.description'), input.description],
    [t('bug_report.steps'), input.steps],
    [t('bug_report.actual'), input.actual],
    [t('bug_report.expected'), input.expected],
  ];
  return sections
    .filter(([, text]) => text !== undefined && text !== '')
    .map(([label, text]) => `**${label}**\n${text}`)
    .join('\n\n');
}

/** The capture as an attachment, or null when none was sent. Refuses anything but JPEG / PNG. */
function screenshotFile(base64: string | undefined): { blob: Blob; name: string } | null {
  if (!base64) return null;
  const bytes = Buffer.from(base64, 'base64');
  const jpeg = bytes.length > 3 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff;
  const png = bytes.length > 8 && bytes.subarray(0, 8).equals(Buffer.from('\x89PNG\r\n\x1a\n', 'latin1'));
  if ((!jpeg && !png) || bytes.length > SCREENSHOT_MAX_BYTES) {
    throw new HttpError(400, 'BUG_REPORT_BAD_SCREENSHOT', 'The capture must be a JPEG or PNG image under 4 MB');
  }
  return {
    blob: new Blob([bytes], { type: jpeg ? 'image/jpeg' : 'image/png' }),
    name: jpeg ? 'screenshot.jpg' : 'screenshot.png',
  };
}

/**
 * The webhook payload of a report. Mentions are switched off: the title and the description
 * are a player's text, and `@everyone` in a bug report must stay text.
 */
export function reportPayload(player: AuthenticatedPlayer, input: BugReportInput, image: string | null): Record<string, unknown> {
  const title = reportTitle(input);
  const fields = Object.entries(input.context ?? {})
    .slice(0, CONTEXT_FIELDS_MAX)
    .map(([name, value]) => ({ name: clip(name, EMBED_TITLE_MAX), value: clip(String(value), EMBED_FIELD_VALUE_MAX) || '-', inline: true }));
  const payload: Record<string, unknown> = {
    allowed_mentions: { parse: [] },
    embeds: [
      {
        title: clip(title, EMBED_TITLE_MAX),
        description: reportBody(input),
        fields,
        footer: { text: `${player.username} · ${player.id}` },
        timestamp: new Date().toISOString(),
        ...(image ? { image: { url: `attachment://${image}` } } : {}),
      },
    ],
  };
  if (env.bugReport.forum) {
    // A forum channel takes a post, not a message: its name is the report's title.
    payload.thread_name = title;
    if (env.bugReport.forumTagIds.length > 0) payload.applied_tags = env.bugReport.forumTagIds;
  }
  return payload;
}

/**
 * Relays a report to Discord.
 * @param player - Who reports (from the token, never from the body).
 * @param input - The validated report.
 * @throws 503 when no webhook is configured, 429 over the limits, 502 when Discord refuses.
 */
export async function submitBugReport(player: AuthenticatedPlayer, input: BugReportInput): Promise<{ delivered: true }> {
  if (!bugReportsEnabled()) {
    throw new HttpError(503, 'BUG_REPORT_DISABLED', 'Bug reports are not relayed on this environment');
  }
  const image = screenshotFile(input.screenshot);
  const now = Date.now();
  takeSlot(player.id, now);

  const form = new FormData();
  form.append('payload_json', JSON.stringify(reportPayload(player, input, image?.name ?? null)));
  let files = 0;
  if (image) form.append(`files[${files++}]`, image.blob, image.name);
  if (input.log) form.append(`files[${files++}]`, new Blob([input.log], { type: 'text/plain' }), 'godot-log.txt');

  let status = 0;
  let detail = '';
  try {
    // `wait=true`: Discord answers once the message exists, so a refusal is seen here.
    const res = await fetch(`${env.bugReport.webhookUrl}?wait=true`, { method: 'POST', body: form });
    status = res.status;
    if (!res.ok) detail = clip(await res.text().catch(() => ''), 300);
  } catch (err) {
    detail = err instanceof Error ? err.message : String(err);
  }
  if (status < 200 || status >= 300) {
    releaseSlot(player.id, now);
    // Never the URL: it is the secret.
    console.error(`Bug report of ${player.id} not relayed: Discord answered ${status || 'nothing'} ${detail}`);
    throw new HttpError(502, 'BUG_REPORT_UPSTREAM', 'The bug report could not be delivered');
  }
  console.log(`Bug report of ${player.id} relayed (${files} file(s))`);
  return { delivered: true };
}
