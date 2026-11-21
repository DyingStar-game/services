// Auth adapter for the text-chat MQTT broker.
//
// mosquitto-go-auth (jwt backend, "remote" mode) calls this service to decide
// whether a connecting client is authenticated and what it may publish/subscribe.
// The client sends its Keycloak access token as the MQTT password; go-auth then
// forwards it to us as "Authorization: Bearer <token>".
//
// We validate the token's signature against Keycloak's JWKS (RS256), so the
// broker never needs the signing secret and key rotation is handled for us.
// ACL is scoped per channel (see `topics.js`): the global chat for everyone,
// DMs for friends, group/corporation chats for members. Membership probes go
// through the Social service internal API (see `social.js`); mutes block
// publishing, bans/suspensions block connecting.

import express from "express";
import { createRemoteJWKSet, jwtVerify } from "jose";

import { createCache } from "./cache.js";
import { createSocialChecks, isSocialConfigured } from "./social.js";
import { classifyAcc, decideAcl } from "./topics.js";

const PORT = Number(process.env.PORT || 3000);
const ISSUER = process.env.OIDC_ISSUER || "http://keycloak:8080/realms/dyingstar";
const JWKS_URL =
  process.env.OIDC_JWKS_URL || `${ISSUER}/protocol/openid-connect/certs`;
const TOPIC_ROOT = process.env.CHAT_TOPIC_ROOT || "chat";
const NOTIFY_ROOT = process.env.CHAT_NOTIFY_ROOT || "notify";
// Legacy allowlist for non-scoped topics; scoped channels (dm/group/corporation)
// are decided by their own rules and can never be re-opened by this list.
const ALLOWED_TOPICS = (
  process.env.CHAT_ALLOWED_TOPICS || `${TOPIC_ROOT}/global,${TOPIC_ROOT}/general`
)
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);
// Service clients (`azp` claim) allowed to publish notifications on {notifyRoot}/#.
const SERVICE_CLIENTS = (process.env.CHAT_SERVICE_CLIENTS || "svc-social")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);
const CACHE_TTL_MS = Number(process.env.CHAT_ACL_CACHE_TTL_SECONDS || 30) * 1000;
const CACHE_MAX = Number(process.env.CHAT_ACL_CACHE_MAX || 10000);

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// Bodies are logged for diagnostics but can carry credentials in form mode, and
// a rogue plugin could send something huge; redact secrets and cap the length.
const BODY_LOG_LIMIT = 400;
const SECRET_KEYS = new Set(["password", "token", "authorization"]);
function redact(value, depth = 0) {
  if (value === null || typeof value !== "object" || depth > 3) return value;
  if (Array.isArray(value)) return value.map((v) => redact(v, depth + 1));
  const out = {};
  for (const [k, v] of Object.entries(value)) {
    out[k] = SECRET_KEYS.has(k.toLowerCase()) ? "<redacted>" : redact(v, depth + 1);
  }
  return out;
}
function describeBody(req) {
  if (req.body === undefined) return "<unparsed>";
  let text;
  try {
    text = JSON.stringify(redact(req.body));
  } catch {
    return "<unserialisable>";
  }
  return text.length > BODY_LOG_LIMIT ? `${text.slice(0, BODY_LOG_LIMIT)}…` : text;
}

const jwks = createRemoteJWKSet(new URL(JWKS_URL));
const cache = createCache({ ttlMs: CACHE_TTL_MS, max: CACHE_MAX });
const checks = createSocialChecks(cache);

// Pull the token from the Authorization header (go-auth) or the password field.
function extractToken(req) {
  const header = req.headers.authorization || "";
  if (header.startsWith("Bearer ")) return header.slice(7);
  return (req.body && req.body.password) || "";
}

async function verifyToken(req) {
  const token = extractToken(req);
  if (!token) return null;
  try {
    const { payload } = await jwtVerify(token, jwks, { issuer: ISSUER });
    if (!payload.sub || !UUID_RE.test(payload.sub)) return null;
    return payload;
  } catch {
    return null;
  }
}

// Whether the token was minted for one of our trusted service clients (client_credentials).
function isServiceToken(payload) {
  const azp = payload.azp || payload.client_id;
  return typeof azp === "string" && SERVICE_CLIENTS.includes(azp);
}

const app = express();

// go-auth logs its own denials but nothing about the calls it makes, so the
// adapter is a black box whenever a check misbehaves ("api error: " with no
// detail is the classic symptom). Log one line per request, whatever it is:
// mounted before the body parsers so requests that never reach a route are
// still visible, and hooked on `finish` so the status is the one actually sent.
app.use((req, res, next) => {
  const startedAt = process.hrtime.bigint();
  res.on("finish", () => {
    const ms = Number(process.hrtime.bigint() - startedAt) / 1e6;
    const auth = req.headers.authorization || "";
    console.log(
      `[chat-auth] ${req.method} ${req.originalUrl} ip=${req.socket.remoteAddress} ` +
        `ct=${req.headers["content-type"] || "-"} clen=${req.headers["content-length"] ?? "-"} ` +
        `auth=${auth ? `present(${auth.length}B)` : "absent"} body=${describeBody(req)} ` +
        `-> ${res.statusCode}${res.locals.bodyParseFailed ? " (body parse failed)" : ""} ` +
        `in ${ms.toFixed(1)}ms`,
    );
  });
  next();
});

// go-auth posts the credential params as the body, but for the user/superuser
// checks it sends a literal `null` (the token travels in the Authorization header,
// not the body). Parse leniently and never fail auth on a malformed body.
app.use(express.json({ strict: false }));
app.use(express.urlencoded({ extended: true }));
app.use((err, _req, res, next) => {
  if (err) {
    res.locals.bodyParseFailed = true;
    return next();
  }
  next();
});

app.get("/health", (_req, res) =>
  res.json({ Ok: true, social: isSocialConfigured() }),
);

// User check: the token must be valid, and a player must not be suspended/banned.
app.post("/user", async (req, res) => {
  const payload = await verifyToken(req);
  if (!payload) return res.json({ Ok: false, Error: "invalid token" });
  if (isServiceToken(payload)) return res.json({ Ok: true, Error: "" });
  if (isSocialConfigured()) {
    try {
      if (await checks.isBlocking(payload.sub)) {
        return res.json({ Ok: false, Error: "suspended or banned" });
      }
    } catch {
      // Social outage: a valid token still connects; mutes remain enforced on publish.
    }
  }
  res.json({ Ok: true, Error: "" });
});

// No superusers. go-auth calls this before *every* acl check, so the refusal is
// normal traffic, not a failure — but it must carry a reason: go-auth logs a
// denial as `api error: <Error>`, and an empty Error is unreadable in the broker
// log. Set auth_opt_disable_superuser true to stop the calls entirely.
app.post("/superuser", (_req, res) => res.json({ Ok: false, Error: "no superusers" }));

// ACL check: valid token AND the topic is allowed for this player.
app.post("/acl", async (req, res) => {
  const payload = await verifyToken(req);
  if (!payload) return res.json({ Ok: false, Error: "invalid token" });

  const topic = (req.body && req.body.topic) || "";
  const access = classifyAcc(req.body && req.body.acc);
  if (!access) return res.json({ Ok: false, Error: "unknown access" });

  try {
    const decision = await decideAcl({
      topic,
      access,
      sub: payload.sub,
      isService: isServiceToken(payload),
      root: TOPIC_ROOT,
      notifyRoot: NOTIFY_ROOT,
      allowedTopics: ALLOWED_TOPICS,
      checks,
    });
    if (!decision.ok) {
      console.log(`[chat-auth] acl deny sub=${payload.sub} acc=${access} topic=${topic}: ${decision.reason}`);
    }
    res.json({ Ok: decision.ok, Error: decision.ok ? "" : decision.reason });
  } catch (err) {
    // Membership probe failed (Social down): fail closed on private channels.
    console.error(`[chat-auth] acl error sub=${payload.sub} topic=${topic}: ${err.message}`);
    res.json({ Ok: false, Error: "check failed" });
  }
});

// An async handler that rejects outside its own try/catch would otherwise take
// the whole adapter down silently, which reads as "the broker stopped asking".
for (const event of ["uncaughtException", "unhandledRejection"]) {
  process.on(event, (err) => {
    console.error(`[chat-auth] ${event}:`, err);
  });
}

app.listen(PORT, () => {
  console.log(
    `[chat-auth] listening on :${PORT} | issuer=${ISSUER} | root=${TOPIC_ROOT} | ` +
      `notify=${NOTIFY_ROOT} | services=${SERVICE_CLIENTS.join(",")} | ` +
      `allowed=${ALLOWED_TOPICS.join(",")} | social=${isSocialConfigured() ? "on" : "OFF"}`
  );
  if (!isSocialConfigured()) {
    console.warn(
      "[chat-auth] CHAT_SOCIAL_API_URL / CHAT_SOCIAL_SERVICE_CLIENT_SECRET not set: " +
        "DM/group/corporation channels are disabled."
    );
  }
});
