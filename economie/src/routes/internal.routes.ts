/**
 * Internal routes for the game server and trusted services (`/api/internal`, `X-Internal-Key`).
 * Wallets are lazily created (like profiles in Social); credits/debits carry an optional
 * `externalId` used as an idempotency key.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { validate } from '../middleware/validate.js';
import {
  ensureCorporationAccount,
  ensurePlayerAccount,
  getCorporationAccounts,
  getPlayerAccounts,
} from '../services/accounts.service.js';
import {
  getCorporationSettings,
  listCorporationMembers,
  removeCorporationMember,
  setCorporationMember,
  updateCorporationSettings,
} from '../services/corporations.service.js';
import {
  creditAccount,
  debitAccount,
  listCorporationTransactions,
  listPlayerTransactions,
} from '../services/transactions.service.js';
import {
  corporationIdParams,
  corporationMemberParams,
  corporationSettingsBody,
  limitQuery,
  memberRoleBody,
  movementBody,
  playerIdParams,
} from './schemas.js';

/** Router for game-server driven updates. */
export const internalRoutes: IRouter = Router();

// ── Player wallets ───────────────────────────────────────────────────────────

/** PUT /players/:playerId/wallet — Ensure a `credits` account exists (login). */
internalRoutes.put(
  '/players/:playerId/wallet',
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await ensurePlayerAccount(req.params.playerId));
  }),
);

/** GET /players/:playerId/wallet — Wallet accounts (one per currency). */
internalRoutes.get(
  '/players/:playerId/wallet',
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json({ accounts: await getPlayerAccounts(req.params.playerId) });
  }),
);

/** GET /players/:playerId/wallet/transactions?limit= — Player ledger. */
internalRoutes.get(
  '/players/:playerId/wallet/transactions',
  validate(playerIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await listPlayerTransactions(req.params.playerId, Number(req.query.limit)));
  }),
);

/** POST /players/:playerId/wallet/credit — Credit (mission reward, salary, etc.). */
internalRoutes.post(
  '/players/:playerId/wallet/credit',
  validate(playerIdParams, 'params'),
  validate(movementBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reference, externalId, type } = req.body;
    const account = await ensurePlayerAccount(req.params.playerId, currency ?? 'credits');
    const result = await creditAccount(account.id, amount, { currency, type, reference, externalId });
    res.status(201).json(result);
  }),
);

/** POST /players/:playerId/wallet/debit — Debit (rejected when balance insufficient). */
internalRoutes.post(
  '/players/:playerId/wallet/debit',
  validate(playerIdParams, 'params'),
  validate(movementBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reference, externalId, type } = req.body;
    const account = await ensurePlayerAccount(req.params.playerId, currency ?? 'credits');
    const result = await debitAccount(account.id, amount, { currency, type, reference, externalId });
    res.status(201).json(result);
  }),
);

// ── Corporation treasuries ───────────────────────────────────────────────────

/** PUT /corporations/:corporationId/wallet — Ensure a treasury `credits` account exists. */
internalRoutes.put(
  '/corporations/:corporationId/wallet',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await ensureCorporationAccount(req.params.corporationId));
  }),
);

/** GET /corporations/:corporationId/wallet — Treasury accounts (one per currency). */
internalRoutes.get(
  '/corporations/:corporationId/wallet',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json({ accounts: await getCorporationAccounts(req.params.corporationId) });
  }),
);

/** GET /corporations/:corporationId/wallet/transactions?limit= — Treasury ledger. */
internalRoutes.get(
  '/corporations/:corporationId/wallet/transactions',
  validate(corporationIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await listCorporationTransactions(req.params.corporationId, Number(req.query.limit)));
  }),
);

/** POST /corporations/:corporationId/wallet/credit — Treasury credit. */
internalRoutes.post(
  '/corporations/:corporationId/wallet/credit',
  validate(corporationIdParams, 'params'),
  validate(movementBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reference, externalId, type } = req.body;
    const account = await ensureCorporationAccount(req.params.corporationId, currency ?? 'credits');
    const result = await creditAccount(account.id, amount, { currency, type, reference, externalId });
    res.status(201).json(result);
  }),
);

/** POST /corporations/:corporationId/wallet/debit — Treasury debit (e.g. payouts). */
internalRoutes.post(
  '/corporations/:corporationId/wallet/debit',
  validate(corporationIdParams, 'params'),
  validate(movementBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reference, externalId, type } = req.body;
    const account = await ensureCorporationAccount(req.params.corporationId, currency ?? 'credits');
    const result = await debitAccount(account.id, amount, { currency, type, reference, externalId });
    res.status(201).json(result);
  }),
);

// ── Corporate membership & settings (mirrors Social's `addNpcCorporationMember`) ──

/** PUT /corporations/:corporationId/members/:playerId {role} — Set a member (and their role). */
internalRoutes.put(
  '/corporations/:corporationId/members/:playerId',
  validate(corporationMemberParams, 'params'),
  validate(memberRoleBody),
  asyncHandler(async (req, res) => {
    const member = await setCorporationMember(req.params.corporationId, req.params.playerId, req.body.role);
    res.json(member);
  }),
);

/** DELETE /corporations/:corporationId/members/:playerId — Remove a member. */
internalRoutes.delete(
  '/corporations/:corporationId/members/:playerId',
  validate(corporationMemberParams, 'params'),
  asyncHandler(async (req, res) => {
    await removeCorporationMember(req.params.corporationId, req.params.playerId);
    res.status(204).send();
  }),
);

/** GET /corporations/:corporationId/members — Treasury staff. */
internalRoutes.get(
  '/corporations/:corporationId/members',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await listCorporationMembers(req.params.corporationId));
  }),
);

/** GET /corporations/:corporationId/settings — Internal tax and donation policy. */
internalRoutes.get(
  '/corporations/:corporationId/settings',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await getCorporationSettings(req.params.corporationId));
  }),
);

/** PUT /corporations/:corporationId/settings — Update internal tax and donation policy. */
internalRoutes.put(
  '/corporations/:corporationId/settings',
  validate(corporationIdParams, 'params'),
  validate(corporationSettingsBody),
  asyncHandler(async (req, res) => {
    const settings = await getCorporationSettings(req.params.corporationId);
    res.json(await updateCorporationSettings(req.params.corporationId, req.body));
  }),
);