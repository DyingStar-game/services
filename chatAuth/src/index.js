// Auth adapter for the text-chat MQTT broker.
//
// mosquitto-go-auth (jwt backend, "remote" mode) calls this service to decide
// whether a connecting client is authenticated and what it may publish/subscribe.
// The client sends its Keycloak access token as the MQTT password; go-auth then
// forwards it to us as "Authorization: Bearer <token>".
//
// We validate the token's signature against Keycloak's JWKS (RS256), so the
// broker never needs the signing secret and key rotation is handled for us. ACL
// is permissive for now (any authenticated player can use the chat topics); it
// will tighten per JWT claims (group/alliance) once those systems exist.

import express from "express";
import { createRemoteJWKSet, jwtVerify } from "jose";

const PORT = Number(process.env.PORT || 3000);
const ISSUER = process.env.OIDC_ISSUER || "http://keycloak:8080/realms/dyingstar";
const JWKS_URL =
  process.env.OIDC_JWKS_URL || `${ISSUER}/protocol/openid-connect/certs`;
// Topic filters any authenticated player may publish/subscribe to (MVP).
const ALLOWED_TOPICS = (process.env.CHAT_ALLOWED_TOPICS || "chat/#")
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);

const jwks = createRemoteJWKSet(new URL(JWKS_URL));

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
    return payload;
  } catch {
    return null;
  }
}

// MQTT topic-filter match (supports + and # wildcards).
function topicMatches(filter, topic) {
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

function topicAllowed(topic) {
  return ALLOWED_TOPICS.some((filter) => topicMatches(filter, topic));
}

const app = express();
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

app.get("/health", (_req, res) => res.json({ Ok: true }));

// User check: the token must be valid.
app.post("/user", async (req, res) => {
  const payload = await verifyToken(req);
  res.json({ Ok: Boolean(payload), Error: payload ? "" : "invalid token" });
});

// No superusers.
app.post("/superuser", (_req, res) => res.json({ Ok: false, Error: "" }));

// ACL check: valid token AND the topic is in the allowed set.
app.post("/acl", async (req, res) => {
  const payload = await verifyToken(req);
  if (!payload) return res.json({ Ok: false, Error: "invalid token" });
  const topic = (req.body && req.body.topic) || "";
  res.json({ Ok: topicAllowed(topic), Error: "" });
});

app.listen(PORT, () => {
  console.log(
    `[chat-auth] listening on :${PORT} | issuer=${ISSUER} | allowed=${ALLOWED_TOPICS.join(",")}`
  );
});
