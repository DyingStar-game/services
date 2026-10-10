import assert from "node:assert/strict";
import { test } from "node:test";

import { classifyAcc, decideAcl, parseChannel, topicMatches } from "../src/topics.js";

const ME = "11111111-1111-4111-8111-111111111111";
const FRIEND = "22222222-2222-4222-8222-222222222222";
const STRANGER = "33333333-3333-4333-8333-333333333333";
const GROUP = "44444444-4444-4444-8444-444444444444";
const CORP = "55555555-5555-4555-8555-555555555555";

const allowAll = {
  isFriend: async () => true,
  inGroup: async () => true,
  inCorp: async () => true,
  isMuted: async () => false,
};

function decide(overrides) {
  return decideAcl({
    topic: "chat/global",
    access: "write",
    sub: ME,
    root: "chat",
    allowedTopics: ["chat/global", "chat/general"],
    checks: allowAll,
    ...overrides,
  });
}

test("classifyAcc maps go-auth bits", () => {
  assert.equal(classifyAcc(1), "subscribe");
  assert.equal(classifyAcc(2), "write");
  assert.equal(classifyAcc(4), "read");
  assert.equal(classifyAcc(8), "subscribe");
  assert.equal(classifyAcc(0), null);
  assert.equal(classifyAcc(undefined), null);
});

test("topicMatches handles wildcards", () => {
  assert.ok(topicMatches("chat/global", "chat/global"));
  assert.ok(topicMatches("chat/#", "chat/dm/a/b"));
  assert.ok(topicMatches("chat/dm/+/x", "chat/dm/y/x"));
  assert.ok(!topicMatches("chat/dm/+/x", "chat/dm/y/z"));
});

test("parseChannel recognises the grammar", () => {
  assert.deepEqual(parseChannel("chat/global", "chat"), { kind: "global" });
  assert.deepEqual(parseChannel(`chat/dm/${ME}/${FRIEND}`, "chat"), {
    kind: "dm", from: ME, to: FRIEND,
  });
  assert.deepEqual(parseChannel(`chat/group/${GROUP}`, "chat"), {
    kind: "group", groupId: GROUP,
  });
  assert.equal(parseChannel("other/topic", "chat"), null);
});

test("global: any player may publish and subscribe", async () => {
  assert.equal((await decide({ topic: "chat/global", access: "write" })).ok, true);
  assert.equal((await decide({ topic: "chat/global", access: "subscribe" })).ok, true);
});

test("global: muted players cannot publish but can still read", async () => {
  const muted = { ...allowAll, isMuted: async () => true };
  assert.equal((await decide({ access: "write", checks: muted })).ok, false);
  assert.equal((await decide({ access: "subscribe", checks: muted })).ok, true);
});

test("legacy allowlist still covers the old general topic", async () => {
  assert.equal((await decide({ topic: "chat/general", access: "write" })).ok, true);
  assert.equal((await decide({ topic: "random/topic", access: "write" })).ok, false);
});

test("dm publish: sender id must match and friendship is required", async () => {
  const topic = `chat/dm/${ME}/${FRIEND}`;
  assert.equal((await decide({ topic, access: "write" })).ok, true);

  const spoof = `chat/dm/${STRANGER}/${FRIEND}`;
  assert.equal((await decide({ topic: spoof, access: "write" })).ok, false);

  const notFriends = {
    ...allowAll,
    isFriend: async () => false,
  };
  assert.equal((await decide({ topic, access: "write", checks: notFriends })).ok, false);

  const self = `chat/dm/${ME}/${ME}`;
  assert.equal((await decide({ topic: self, access: "write" })).ok, false);
});

test("dm publish: muted sender is denied", async () => {
  const muted = { ...allowAll, isMuted: async () => true };
  const topic = `chat/dm/${ME}/${FRIEND}`;
  assert.equal((await decide({ topic, access: "write", checks: muted })).ok, false);
});

test("dm receive: recipient subscribes once with dm/+/{me}", async () => {
  assert.equal(
    (await decide({ topic: `chat/dm/+/${ME}`, access: "subscribe" })).ok,
    true,
  );
  assert.equal(
    (await decide({ topic: `chat/dm/${FRIEND}/${ME}`, access: "read" })).ok,
    true,
  );
});

test("dm receive: anyone else is denied", async () => {
  assert.equal(
    (await decide({ topic: `chat/dm/+/${FRIEND}`, access: "subscribe" })).ok,
    false,
  );
  assert.equal(
    (await decide({ topic: `chat/dm/+/${STRANGER}/${ME}`, access: "subscribe" })).ok,
    false,
  );
  assert.equal(
    (await decide({ topic: "chat/dm/+/+", access: "subscribe" })).ok,
    false,
  );
});

test("dm: a wide allowlist cannot re-open private channels", async () => {
  const notFriends = { ...allowAll, isFriend: async () => false };
  const decision = await decide({
    topic: `chat/dm/${ME}/${FRIEND}`,
    access: "write",
    allowedTopics: ["chat/#"],
    checks: notFriends,
  });
  assert.equal(decision.ok, false);
});

test("group: members only, publish and subscribe", async () => {
  const topic = `chat/group/${GROUP}`;
  assert.equal((await decide({ topic, access: "write" })).ok, true);
  assert.equal((await decide({ topic, access: "subscribe" })).ok, true);

  const outsider = { ...allowAll, inGroup: async () => false };
  assert.equal((await decide({ topic, access: "write", checks: outsider })).ok, false);
  assert.equal((await decide({ topic, access: "subscribe", checks: outsider })).ok, false);
});

test("group: no wildcards, valid uuid only", async () => {
  assert.equal(
    (await decide({ topic: "chat/group/+", access: "subscribe" })).ok,
    false,
  );
  assert.equal(
    (await decide({ topic: "chat/group/#", access: "subscribe" })).ok,
    false,
  );
});

test("corporation: members only", async () => {
  const topic = `chat/corporation/${CORP}`;
  assert.equal((await decide({ topic, access: "write" })).ok, true);

  const outsider = { ...allowAll, inCorp: async () => false };
  assert.equal((await decide({ topic, access: "subscribe", checks: outsider })).ok, false);
});

test("membership probe failure fails closed", async () => {
  const broken = {
    ...allowAll,
    inGroup: async () => {
      throw new Error("social down");
    },
  };
  await assert.rejects(
    decide({ topic: `chat/group/${GROUP}`, access: "write", checks: broken }),
    /social down/,
  );
});

test("missing player id is denied", async () => {
  assert.equal((await decide({ sub: null })).ok, false);
  assert.equal((await decide({ sub: "not-a-uuid" })).ok, false);
});

// ── Notification channel ────────────────────────────────────────────────────

test("notify: a player subscribes to their own channel only", async () => {
  assert.equal(
    (await decide({ topic: `notify/${ME}`, access: "subscribe" })).ok,
    true,
  );
  assert.equal(
    (await decide({ topic: `notify/${ME}`, access: "read" })).ok,
    true,
  );
  assert.equal(
    (await decide({ topic: `notify/${FRIEND}`, access: "subscribe" })).ok,
    false,
  );
  assert.equal(
    (await decide({ topic: "notify/+", access: "subscribe" })).ok,
    false,
  );
  assert.equal(
    (await decide({ topic: "notify/#", access: "subscribe" })).ok,
    false,
  );
});

test("notify: a player cannot publish notifications", async () => {
  assert.equal(
    (await decide({ topic: `notify/${ME}`, access: "write" })).ok,
    false,
  );
});

test("notify: service clients publish to any player, never subscribe", async () => {
  const service = { sub: ME, isService: true };
  assert.equal(
    (await decide({ ...service, topic: `notify/${FRIEND}`, access: "write" })).ok,
    true,
  );
  assert.equal(
    (await decide({ ...service, topic: `notify/${STRANGER}`, access: "write" })).ok,
    true,
  );
  assert.equal(
    (await decide({ ...service, topic: "notify/not-a-uuid", access: "write" })).ok,
    false,
  );
  assert.equal(
    (await decide({ ...service, topic: "notify/#", access: "subscribe" })).ok,
    false,
  );
});

test("service clients get no access to player channels", async () => {
  const service = { sub: ME, isService: true };
  assert.equal(
    (await decide({ ...service, topic: "chat/global", access: "write" })).ok,
    false,
  );
  assert.equal(
    (await decide({ ...service, topic: `chat/dm/${ME}/${FRIEND}`, access: "write" })).ok,
    false,
  );
  assert.equal(
    (await decide({ ...service, topic: `chat/group/${GROUP}`, access: "subscribe" })).ok,
    false,
  );
});
