// Topic grammar and ACL decisions for the text-chat broker.
//
// Topic layout (root configurable, default "chat"):
//   {root}/global                       — any authenticated player
//   {root}/dm/{fromId}/{toId}           — direct message; only the pair may touch it
//   {root}/group/{groupId}              — group chat; members only
//   {root}/corporation/{corporationId}  — corporation chat; members only
//
// The scoped rules (dm/group/corporation) always win over the legacy allowlist, so
// widening CHAT_ALLOWED_TOPICS can never re-open private channels.

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** MQTT ACL access bits as sent by mosquitto-go-auth. */
export const ACC = {
  SUBSCRIBE: 0x01,
  WRITE: 0x02,
  READ: 0x04,
  UNSUBSCRIBE: 0x08,
};

/** Classifies an `acc` value; `null` when the access is unknown (deny). */
export function classifyAcc(acc) {
  const bits = Number(acc) || 0;
  if (bits & ACC.WRITE) return "write";
  if (bits & (ACC.SUBSCRIBE | ACC.UNSUBSCRIBE)) return "subscribe";
  if (bits & ACC.READ) return "read";
  return null;
}

/**
 * Parses a topic (or topic filter) into a scoped channel, or null when the topic
 * does not belong to the private grammar.
 */
export function parseChannel(topic, root) {
  const parts = topic.split("/");
  if (parts[0] !== root) return null;
  if (parts.length === 2 && parts[1] === "global") return { kind: "global" };
  if (parts.length === 4 && parts[1] === "dm") {
    return { kind: "dm", from: parts[2], to: parts[3] };
  }
  if (parts.length === 3 && parts[1] === "group") {
    return { kind: "group", groupId: parts[2] };
  }
  if (parts.length === 3 && parts[1] === "corporation") {
    return { kind: "corporation", corporationId: parts[2] };
  }
  return null;
}

/** MQTT topic-filter match (supports + and # wildcards). */
export function topicMatches(filter, topic) {
  if (filter === "#") return true;
  const f = filter.split("/");
  const t = topic.split("/");
  for (let i = 0; i < f.length; i++) {
    if (f[i] === "#") return true;
    if (f[i] === "+") {
      if (t[i] === undefined) return false;
      continue;
    }
    if (f[i] !== t[i]) return false;
  }
  return f.length === t.length;
}

/**
 * Decides whether `sub` (the verified player id) may perform `access` on `topic`.
 *
 * @param input.topic        MQTT topic or topic filter.
 * @param input.access       "write" | "subscribe" | "read".
 * @param input.sub          Verified player id (JWT `sub`).
 * @param input.root         Topic root, default "chat".
 * @param input.allowedTopics Legacy allowlist filters (global chat).
 * @param input.checks       Async membership probes, see `social.js`:
 *                           isFriend(a,b), inGroup(id,groupId), inCorp(id,corpId),
 *                           isMuted(id). Membership probes throw on transport error
 *                           (fail-closed); isMuted degrades to "not muted".
 * @returns { ok: boolean, reason: string }
 */
export async function decideAcl({
  topic,
  access,
  sub,
  root = "chat",
  allowedTopics = [],
  checks,
}) {
  if (!sub || !UUID_RE.test(sub)) return { ok: false, reason: "no player id" };
  if (!topic) return { ok: false, reason: "empty topic" };

  const channel = parseChannel(topic, root);

  // Scoped channels: private rules apply even if the legacy allowlist is wide open.
  if (channel) {
    if (channel.kind === "global") {
      if (access === "write" && (await checks.isMuted(sub))) {
        return { ok: false, reason: "muted" };
      }
      return { ok: true, reason: "global" };
    }

    if (channel.kind === "dm") {
      const { from, to } = channel;
      if (access === "write") {
        if (!UUID_RE.test(from) || !UUID_RE.test(to)) {
          return { ok: false, reason: "invalid dm ids" };
        }
        if (from !== sub) return { ok: false, reason: "cannot impersonate sender" };
        if (to === sub) return { ok: false, reason: "no self dm" };
        if (!(await checks.isFriend(from, to))) return { ok: false, reason: "not friends" };
        if (await checks.isMuted(sub)) return { ok: false, reason: "muted" };
        return { ok: true, reason: "dm publish" };
      }
      // read/subscribe: only the recipient may listen, either with the concrete
      // topic or the canonical filter `dm/+/{to}` (one subscription for all senders).
      if (!UUID_RE.test(to) || to !== sub) return { ok: false, reason: "not the recipient" };
      if (from !== "+" && !UUID_RE.test(from)) {
        return { ok: false, reason: "invalid dm ids" };
      }
      return { ok: true, reason: "dm receive" };
    }

    if (channel.kind === "group") {
      if (!UUID_RE.test(channel.groupId)) return { ok: false, reason: "invalid group id" };
      if (access === "write") {
        if (await checks.isMuted(sub)) return { ok: false, reason: "muted" };
      }
      if (!(await checks.inGroup(sub, channel.groupId))) {
        return { ok: false, reason: "not in group" };
      }
      return { ok: true, reason: "group" };
    }

    if (channel.kind === "corporation") {
      if (!UUID_RE.test(channel.corporationId)) {
        return { ok: false, reason: "invalid corporation id" };
      }
      if (access === "write") {
        if (await checks.isMuted(sub)) return { ok: false, reason: "muted" };
      }
      if (!(await checks.inCorp(sub, channel.corporationId))) {
        return { ok: false, reason: "not in corporation" };
      }
      return { ok: true, reason: "corporation" };
    }
  }

  // Legacy allowlist (covers {root}/global and any pre-scoped topic).
  if (allowedTopics.some((filter) => topicMatches(filter, topic))) {
    if (access === "write" && (await checks.isMuted(sub))) {
      return { ok: false, reason: "muted" };
    }
    return { ok: true, reason: "allowlist" };
  }

  return { ok: false, reason: "unknown topic" };
}
