/**
 * The bugs already posted in the QA forum, so a player can add to one instead of opening a second
 * post for the same bug.
 *
 * A webhook can write to a forum but cannot read it: listing the posts takes a Discord BOT
 * (`DISCORD_BUG_BOT_TOKEN`, a secret, with "View Channel" and "Read Message History" on the
 * forum). The forum is the one the webhook posts to: Discord says which when asked about the
 * webhook, so nothing else is configured. Without the token the list is empty and off.
 *
 * Where a bug stands is the square the QA team puts as a reaction under its post: red (open),
 * orange (a developer took it), purple (fixed, to be tested in the build), green (closed). A post
 * with no square is new. Closed and archived posts are left out of the list.
 *
 * Nothing is stored: the list is read from Discord and kept a couple of minutes in memory.
 */
import { env } from '../config/env.js';

const DISCORD_API = 'https://discord.com/api/v10';
/** How long a list read from Discord is served again (ms). */
const LIST_TTL_MS = 120_000;
/** Posts read at most, newest first, and how many of their first messages are asked at once. */
const THREADS_MAX = 150;
const READS_AT_ONCE = 5;

/** Where a bug stands, in the order it goes through them. */
export const BUG_STATUSES = ['new', 'open', 'in_progress', 'to_test', 'closed'] as const;
export type BugStatus = (typeof BUG_STATUSES)[number];
/**
 * The square of each status, the furthest first: a post that kept its red square under a green
 * one is closed.
 */
const STATUS_MARKS: [string, BugStatus][] = [
  ['🟩', 'closed'],
  ['🟪', 'to_test'],
  ['🟧', 'in_progress'],
  ['🟥', 'open'],
];

export interface KnownBug {
  /** The post's id in Discord: what a report names to be added to it. */
  id: string;
  /** The post's name, as the QA forum lists it. */
  title: string;
  status: BugStatus;
}

interface DiscordReaction {
  count?: number;
  emoji?: { name?: string | null };
}

interface DiscordThread {
  id: string;
  name?: string;
  parent_id?: string | null;
  thread_metadata?: { archived?: boolean; locked?: boolean };
}

let forum: { channelId: string; guildId: string } | null = null;
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

/** Where a post stands, from the reactions under it. Pure. */
export function statusOf(reactions: DiscordReaction[] | undefined): BugStatus {
  const present = new Set((reactions ?? []).filter((reaction) => (reaction.count ?? 0) > 0).map((reaction) => reaction.emoji?.name));
  return STATUS_MARKS.find(([mark]) => present.has(mark))?.[1] ?? 'new';
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

/** The forum the webhook posts to. Asked once: a webhook does not change channel under us. */
async function forumOfWebhook(): Promise<{ channelId: string; guildId: string }> {
  if (forum) return forum;
  // The webhook's own URL answers without a bot; never logged, it is the secret.
  const res = await fetch(env.bugReport.webhookUrl);
  if (!res.ok) throw new Error(`Discord answered ${res.status} about the webhook`);
  const webhook = (await res.json()) as { channel_id?: string; guild_id?: string };
  if (!webhook.channel_id || !webhook.guild_id) throw new Error('The webhook names no channel');
  forum = { channelId: webhook.channel_id, guildId: webhook.guild_id };
  return forum;
}

async function readBugs(): Promise<KnownBug[]> {
  const { channelId, guildId } = await forumOfWebhook();
  const active = await discord<{ threads?: DiscordThread[] }>(`/guilds/${guildId}/threads/active`);
  const posts = (active.threads ?? [])
    .filter((thread) => thread.parent_id === channelId && !thread.thread_metadata?.archived && !thread.thread_metadata?.locked)
    .sort(newestFirst)
    .slice(0, THREADS_MAX);
  const bugs: KnownBug[] = [];
  for (let from = 0; from < posts.length; from += READS_AT_ONCE) {
    const batch = posts.slice(from, from + READS_AT_ONCE);
    const read = await Promise.all(
      batch.map(async (post): Promise<KnownBug> => {
        // In a forum the first message of a post has the post's id; the squares are under it.
        const first = await discord<{ reactions?: DiscordReaction[] }>(`/channels/${post.id}/messages/${post.id}`).catch(() => null);
        return { id: post.id, title: post.name ?? '', status: statusOf(first?.reactions) };
      }),
    );
    bugs.push(...read);
  }
  return bugs;
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
