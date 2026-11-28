/**
 * The bugs already posted in the QA forum, so a player can add to one instead of opening a second
 * post for the same bug.
 *
 * A webhook can write to a forum but cannot read it: listing the posts takes a Discord BOT
 * (`DISCORD_BUG_BOT_TOKEN`, a secret, with "View Channel" on the forum). The forum is the one the
 * webhook posts to: Discord says which when asked about the webhook, so nothing else is
 * configured. Without the token the list is empty and off.
 *
 * Where a bug stands is the TAG the QA team puts on its post, known by the square that is the
 * tag's emoji, whatever its name: red (open, nobody took it), orange (a developer took it), blue
 * (to be tested: reports can still add to it), green (fixed), boxed square (a duplicate, or one
 * that will not be fixed). The last two are closed. A post with none of them is new. Closed and
 * archived posts are left out of the list. Tags replaced the squares the team used to put as
 * reactions (2026-11-27): a post's tags come with the list of posts, where a reaction took one
 * more request per post.
 *
 * Nothing is stored: the list is read from Discord and kept a couple of minutes in memory.
 */
import { env } from '../config/env.js';

const DISCORD_API = 'https://discord.com/api/v10';
/** How long a list read from Discord is served again (ms). */
const LIST_TTL_MS = 120_000;
/** Posts listed at most, newest first. */
const THREADS_MAX = 150;

/** Where a bug stands, in the order it goes through them. */
export const BUG_STATUSES = ['new', 'open', 'in_progress', 'to_test', 'closed'] as const;
export type BugStatus = (typeof BUG_STATUSES)[number];
/**
 * The square of each status, the furthest first: a post that kept its red tag beside a green one
 * is closed.
 */
const STATUS_MARKS: [string, BugStatus][] = [
  ['🔳', 'closed'],
  ['🟩', 'closed'],
  ['🟦', 'to_test'],
  ['🟧', 'in_progress'],
  ['🟥', 'open'],
];
/** The square of the tag a new report is posted under: open, nobody took it yet. */
const NEW_REPORT_MARK = '🟥';
/** What opens a post's name and is not the bug's own words: the severity's round mark, then bracketed labels. */
const TITLE_PREFIX = /^\s*(?:[🔴🟠🟡🟣]\s*)?(?:\[[^\]]*\]\s*)*/u;

export interface KnownBug {
  /** The post's id in Discord: what a report names to be added to it. */
  id: string;
  /** The post's name without its mark and labels (plainTitle). */
  title: string;
  status: BugStatus;
}

interface DiscordTag {
  id: string;
  emoji_name?: string | null;
}

interface DiscordThread {
  id: string;
  name?: string;
  parent_id?: string | null;
  applied_tags?: string[];
  thread_metadata?: { archived?: boolean; locked?: boolean };
}

/** The forum the webhook posts to, and what its tags stand for. */
interface Forum {
  channelId: string;
  guildId: string;
  /** The status of each tag that is one, by tag id. */
  statusOfTag: Map<string, BugStatus>;
  /** The tag of a report nobody took yet, when the forum has one. */
  newReportTagId: string | null;
}

let forum: Forum | null = null;
let cached: { at: number; bugs: KnownBug[] } | null = null;
let reading: Promise<KnownBug[]> | null = null;

/** Whether this environment can list the bugs already posted. */
export function bugListEnabled(): boolean {
  return env.bugReport.webhookUrl !== '' && env.bugReport.botToken !== '';
}

/** Forgets the forum and the list (tests). */
export function resetBugList(): void {
  forum = null;
  cached = null;
  reading = null;
}

/** The status each tag of [tags] stands for, by tag id: those whose emoji is a status square. Pure. */
export function statusTags(tags: DiscordTag[] | undefined): Map<string, BugStatus> {
  const known = new Map(STATUS_MARKS);
  const out = new Map<string, BugStatus>();
  for (const tag of tags ?? []) {
    const status = known.get(tag.emoji_name ?? '');
    if (status) out.set(tag.id, status);
  }
  return out;
}

/** Where a post stands, from the tags it carries: the furthest of them, new with none. Pure. */
export function statusOf(appliedTags: string[] | undefined, statusOfTag: ReadonlyMap<string, BugStatus>): BugStatus {
  const present = new Set((appliedTags ?? []).map((id) => statusOfTag.get(id)));
  return STATUS_MARKS.find(([, status]) => present.has(status))?.[1] ?? 'new';
}

/**
 * A post's name as the game lists it: the bug's own words, without the severity mark and the
 * labels ahead of them (`🟡 [PREPROD] Stuck` -> `Stuck`), which hid the words a player looks a
 * duplicate up by. A name made of labels only is kept whole. Pure.
 */
export function plainTitle(name: string): string {
  return name.replace(TITLE_PREFIX, '').trim() || name.trim();
}

/** A Discord id is a number that grows with time: the newest first. */
function newestFirst(a: { id: string }, b: { id: string }): number {
  if (a.id.length !== b.id.length) return b.id.length - a.id.length;
  return a.id < b.id ? 1 : a.id > b.id ? -1 : 0;
}

async function discord<T>(path: string): Promise<T> {
  const res = await fetch(`${DISCORD_API}${path}`, { headers: { authorization: `Bot ${env.bugReport.botToken}` } });
  if (!res.ok) throw new Error(`Discord answered ${res.status} to ${path}`);
  return (await res.json()) as T;
}

/**
 * The forum the webhook posts to, with its tags. Asked once: a webhook does not change channel
 * under us, and the tags are the QA team's process, set once (a restart reads them again).
 */
async function forumOfWebhook(): Promise<Forum> {
  if (forum) return forum;
  // The webhook's own URL answers without a bot; never logged, it is the secret.
  const res = await fetch(env.bugReport.webhookUrl);
  if (!res.ok) throw new Error(`Discord answered ${res.status} about the webhook`);
  const webhook = (await res.json()) as { channel_id?: string; guild_id?: string };
  if (!webhook.channel_id || !webhook.guild_id) throw new Error('The webhook names no channel');
  const channel = await discord<{ available_tags?: DiscordTag[] }>(`/channels/${webhook.channel_id}`);
  forum = {
    channelId: webhook.channel_id,
    guildId: webhook.guild_id,
    statusOfTag: statusTags(channel.available_tags),
    newReportTagId: channel.available_tags?.find((tag) => tag.emoji_name === NEW_REPORT_MARK)?.id ?? null,
  };
  return forum;
}

/**
 * The tag a new report is posted under (open, nobody took it); null where no bot reads the
 * forum's tags, where the forum has no such tag, or when Discord cannot be read: a report is then
 * posted without it rather than not at all.
 */
export async function newReportTagId(): Promise<string | null> {
  if (!bugListEnabled()) return null;
  try {
    return (await forumOfWebhook()).newReportTagId;
  } catch (err) {
    console.error(`Bug forum tags not read from Discord: ${err instanceof Error ? err.message : String(err)}`);
    return null;
  }
}

async function readBugs(): Promise<KnownBug[]> {
  const { channelId, guildId, statusOfTag } = await forumOfWebhook();
  const active = await discord<{ threads?: DiscordThread[] }>(`/guilds/${guildId}/threads/active`);
  return (active.threads ?? [])
    .filter((thread) => thread.parent_id === channelId && !thread.thread_metadata?.archived && !thread.thread_metadata?.locked)
    .sort(newestFirst)
    .slice(0, THREADS_MAX)
    .map((post) => ({ id: post.id, title: plainTitle(post.name ?? ''), status: statusOf(post.applied_tags, statusOfTag) }));
}

/**
 * Every post of the forum still active, with where it stands, newest first. Served from memory
 * for LIST_TTL_MS; when Discord cannot be read, the last list known (or none) is served instead.
 */
export async function knownBugs(): Promise<KnownBug[]> {
  if (!bugListEnabled()) return [];
  if (cached && Date.now() - cached.at < LIST_TTL_MS) return cached.bugs;
  if (!reading) {
    reading = readBugs()
      .then((bugs) => {
        cached = { at: Date.now(), bugs };
        return bugs;
      })
      .catch((err: unknown) => {
        console.error(`Bug list not read from Discord: ${err instanceof Error ? err.message : String(err)}`);
        return cached?.bugs ?? [];
      })
      .finally(() => {
        reading = null;
      });
  }
  return reading;
}

/** The bugs a report may be added to: every one that is not closed. */
export async function openBugs(): Promise<KnownBug[]> {
  return (await knownBugs()).filter((bug) => bug.status !== 'closed');
}
