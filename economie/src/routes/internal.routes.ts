/**
 * Internal routes for the game server and trusted services (`/api/internal`). The caller is
 * authenticated by `serviceAuth` (Keycloak service account) on the mount point, then each
 * route requires its capability role. Wallets are lazily created (like profiles in Social);
 * credits/debits carry an optional `externalId` used as an idempotency key.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requireService, requireServiceRole, SERVICE_ROLES } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import {
  ensureCorporationAccount,
  ensureNpcAccount,
  ensurePlayerAccount,
  ensurePoliticalAccount,
  getCorporationAccounts,
  getNpcAccounts,
  getPlayerAccounts,
  getPoliticalAccounts,
} from '../services/accounts.service.js';
import {
  getCorporationSettings,
  listCorporationMembers,
  removeCorporationMember,
  setCorporationMember,
  updateCorporationSettings,
} from '../services/corporations.service.js';
import {
  getPoliticalSettings,
  getTaxDebts,
  issueCurrency,
  listPoliticalMembers,
  removePoliticalMember,
  setPoliticalMember,
  updatePoliticalSettings,
} from '../services/politics.service.js';
import { assessTaxes, payTaxDebts } from '../services/taxation.service.js';
import {
  creditAccount,
  debitAccount,
  listCorporationTransactions,
  listNpcTransactions,
  listPlayerTransactions,
  listPoliticalTransactions,
} from '../services/transactions.service.js';
import {
  affiliationBody,
  assessBody,
  corporationIdParams,
  corporationMemberParams,
  corporationSettingsBody,
  limitQuery,
  memberRoleBody,
  mintBody,
  movementBody,
  npcIdParams,
  payTaxesBody,
  playerIdParams,
  politicalEntityIdParams,
  politicalMemberParams,
  politicalMemberRoleBody,
  politicalSettingsBody,
} from './schemas.js';

/** Router for game-server driven updates. */
export const internalRoutes: IRouter = Router();

// ── Player wallets ───────────────────────────────────────────────────────────

/** PUT /players/:playerId/wallet — Ensure a `credits` account exists (login). */
internalRoutes.put(
  '/players/:playerId/wallet',
  requireServiceRole(SERVICE_ROLES.walletEnsure),
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await ensurePlayerAccount(req.params.playerId));
  }),
);

/** GET /players/:playerId/wallet — Wallet accounts (one per currency). */
internalRoutes.get(
  '/players/:playerId/wallet',
  requireServiceRole(SERVICE_ROLES.walletRead),
  validate(playerIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json({ accounts: await getPlayerAccounts(req.params.playerId) });
  }),
);

/** GET /players/:playerId/wallet/transactions?limit=&offset= — Player ledger. */
internalRoutes.get(
  '/players/:playerId/wallet/transactions',
  requireServiceRole(SERVICE_ROLES.walletRead),
  validate(playerIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await listPlayerTransactions(req.params.playerId, Number(req.query.limit), Number(req.query.offset)));
  }),
);

/** POST /players/:playerId/wallet/credit — Credit (mission reward, salary, etc.). */
internalRoutes.post(
  '/players/:playerId/wallet/credit',
  requireServiceRole(SERVICE_ROLES.walletCredit),
  validate(playerIdParams, 'params'),
  validate(movementBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reference, externalId, type } = req.body;
    const account = await ensurePlayerAccount(req.params.playerId, currency ?? 'credits');
    const result = await creditAccount(account.id, amount, {
      currency,
      type,
      reference,
      externalId,
      caller: requireService(req).clientId,
    });
    res.status(201).json(result);
  }),
);

/** POST /players/:playerId/wallet/debit — Debit (rejected when balance insufficient). */
internalRoutes.post(
  '/players/:playerId/wallet/debit',
  requireServiceRole(SERVICE_ROLES.walletDebit),
  validate(playerIdParams, 'params'),
  validate(movementBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reference, externalId, type } = req.body;
    const account = await ensurePlayerAccount(req.params.playerId, currency ?? 'credits');
    const result = await debitAccount(account.id, amount, {
      currency,
      type,
      reference,
      externalId,
      caller: requireService(req).clientId,
    });
    res.status(201).json(result);
  }),
);

// ── NPC wallets ──────────────────────────────────────────────────────────────

/** PUT /npcs/:npcId/wallet — Ensure an NPC's `credits` account exists. */
internalRoutes.put(
  '/npcs/:npcId/wallet',
  requireServiceRole(SERVICE_ROLES.walletEnsure),
  validate(npcIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await ensureNpcAccount(req.params.npcId));
  }),
);

/** GET /npcs/:npcId/wallet — NPC wallet accounts (one per currency). */
internalRoutes.get(
  '/npcs/:npcId/wallet',
  requireServiceRole(SERVICE_ROLES.walletRead),
  validate(npcIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json({ accounts: await getNpcAccounts(req.params.npcId) });
  }),
);

/** GET /npcs/:npcId/wallet/transactions?limit=&offset= — NPC ledger. */
internalRoutes.get(
  '/npcs/:npcId/wallet/transactions',
  requireServiceRole(SERVICE_ROLES.walletRead),
  validate(npcIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await listNpcTransactions(req.params.npcId, Number(req.query.limit), Number(req.query.offset)));
  }),
);

/** POST /npcs/:npcId/wallet/credit — Credit an NPC (salary, prime, sale, etc.). */
internalRoutes.post(
  '/npcs/:npcId/wallet/credit',
  requireServiceRole(SERVICE_ROLES.walletCredit),
  validate(npcIdParams, 'params'),
  validate(movementBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reference, externalId, type } = req.body;
    const account = await ensureNpcAccount(req.params.npcId, currency ?? 'credits');
    const result = await creditAccount(account.id, amount, {
      currency,
      type,
      reference,
      externalId,
      caller: requireService(req).clientId,
    });
    res.status(201).json(result);
  }),
);

/** POST /npcs/:npcId/wallet/debit — Debit an NPC (rejected when balance insufficient). */
internalRoutes.post(
  '/npcs/:npcId/wallet/debit',
  requireServiceRole(SERVICE_ROLES.walletDebit),
  validate(npcIdParams, 'params'),
  validate(movementBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reference, externalId, type } = req.body;
    const account = await ensureNpcAccount(req.params.npcId, currency ?? 'credits');
    const result = await debitAccount(account.id, amount, {
      currency,
      type,
      reference,
      externalId,
      caller: requireService(req).clientId,
    });
    res.status(201).json(result);
  }),
);

// ── Corporation treasuries ───────────────────────────────────────────────────

/** PUT /corporations/:corporationId/wallet — Ensure a treasury `credits` account exists. */
internalRoutes.put(
  '/corporations/:corporationId/wallet',
  requireServiceRole(SERVICE_ROLES.walletEnsure),
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await ensureCorporationAccount(req.params.corporationId));
  }),
);

/** GET /corporations/:corporationId/wallet — Treasury accounts (one per currency). */
internalRoutes.get(
  '/corporations/:corporationId/wallet',
  requireServiceRole(SERVICE_ROLES.walletRead),
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json({ accounts: await getCorporationAccounts(req.params.corporationId) });
  }),
);

/** GET /corporations/:corporationId/wallet/transactions?limit=&offset= — Treasury ledger. */
internalRoutes.get(
  '/corporations/:corporationId/wallet/transactions',
  requireServiceRole(SERVICE_ROLES.walletRead),
  validate(corporationIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await listCorporationTransactions(req.params.corporationId, Number(req.query.limit), Number(req.query.offset)));
  }),
);

/** POST /corporations/:corporationId/wallet/credit — Treasury credit. */
internalRoutes.post(
  '/corporations/:corporationId/wallet/credit',
  requireServiceRole(SERVICE_ROLES.walletCredit),
  validate(corporationIdParams, 'params'),
  validate(movementBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reference, externalId, type } = req.body;
    const account = await ensureCorporationAccount(req.params.corporationId, currency ?? 'credits');
    const result = await creditAccount(account.id, amount, {
      currency,
      type,
      reference,
      externalId,
      caller: requireService(req).clientId,
    });
    res.status(201).json(result);
  }),
);

/** POST /corporations/:corporationId/wallet/debit — Treasury debit (e.g. payouts). */
internalRoutes.post(
  '/corporations/:corporationId/wallet/debit',
  requireServiceRole(SERVICE_ROLES.walletDebit),
  validate(corporationIdParams, 'params'),
  validate(movementBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reference, externalId, type } = req.body;
    const account = await ensureCorporationAccount(req.params.corporationId, currency ?? 'credits');
    const result = await debitAccount(account.id, amount, {
      currency,
      type,
      reference,
      externalId,
      caller: requireService(req).clientId,
    });
    res.status(201).json(result);
  }),
);

// ── Corporate membership & settings (mirrors Social's `addNpcCorporationMember`) ──

/** PUT /corporations/:corporationId/members/:playerId {role} — Set a member (and their role). */
internalRoutes.put(
  '/corporations/:corporationId/members/:playerId',
  requireServiceRole(SERVICE_ROLES.corporationManage),
  validate(corporationMemberParams, 'params'),
  validate(memberRoleBody),
  asyncHandler(async (req, res) => {
    const member = await setCorporationMember(
      req.params.corporationId,
      req.params.playerId,
      req.body.role,
      req.body.holderType,
    );
    res.json(member);
  }),
);

/** DELETE /corporations/:corporationId/members/:playerId — Remove a member. */
internalRoutes.delete(
  '/corporations/:corporationId/members/:playerId',
  requireServiceRole(SERVICE_ROLES.corporationManage),
  validate(corporationMemberParams, 'params'),
  asyncHandler(async (req, res) => {
    await removeCorporationMember(req.params.corporationId, req.params.playerId);
    res.status(204).send();
  }),
);

/** GET /corporations/:corporationId/members?limit=&offset= — Treasury staff. */
internalRoutes.get(
  '/corporations/:corporationId/members',
  requireServiceRole(SERVICE_ROLES.corporationRead),
  validate(corporationIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await listCorporationMembers(req.params.corporationId, Number(req.query.limit), Number(req.query.offset)));
  }),
);

/** GET /corporations/:corporationId/settings — Internal tax and donation policy. */
internalRoutes.get(
  '/corporations/:corporationId/settings',
  requireServiceRole(SERVICE_ROLES.corporationRead),
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await getCorporationSettings(req.params.corporationId));
  }),
);

/** PUT /corporations/:corporationId/settings — Update internal tax and donation policy. */
internalRoutes.put(
  '/corporations/:corporationId/settings',
  requireServiceRole(SERVICE_ROLES.corporationManage),
  validate(corporationIdParams, 'params'),
  validate(corporationSettingsBody),
  asyncHandler(async (req, res) => {
    await getCorporationSettings(req.params.corporationId);
    res.json(await updateCorporationSettings(req.params.corporationId, req.body));
  }),
);

/** PUT /corporations/:corporationId/affiliation — Set the corporation's political (fiscal) home. */
internalRoutes.put(
  '/corporations/:corporationId/affiliation',
  requireServiceRole(SERVICE_ROLES.corporationManage),
  validate(corporationIdParams, 'params'),
  validate(affiliationBody),
  asyncHandler(async (req, res) => {
    await getCorporationSettings(req.params.corporationId);
    res.json(await updateCorporationSettings(req.params.corporationId, { politicalEntityId: req.body.politicalEntityId }));
  }),
);

// ── Political treasuries, taxes & minting ────────────────────────────────────

/** PUT /politics/:entityId/wallet — Ensure a political treasury `credits` account exists. */
internalRoutes.put(
  '/politics/:entityId/wallet',
  requireServiceRole(SERVICE_ROLES.walletEnsure),
  validate(politicalEntityIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await ensurePoliticalAccount(req.params.entityId));
  }),
);

/** GET /politics/:entityId/wallet — Treasury accounts (one per currency). */
internalRoutes.get(
  '/politics/:entityId/wallet',
  requireServiceRole(SERVICE_ROLES.politicsRead),
  validate(politicalEntityIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json({ accounts: await getPoliticalAccounts(req.params.entityId) });
  }),
);

/** GET /politics/:entityId/wallet/transactions?limit=&offset= — Treasury ledger. */
internalRoutes.get(
  '/politics/:entityId/wallet/transactions',
  requireServiceRole(SERVICE_ROLES.politicsRead),
  validate(politicalEntityIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await listPoliticalTransactions(req.params.entityId, Number(req.query.limit), Number(req.query.offset)));
  }),
);

/** POST /politics/:entityId/wallet/credit — Treasury credit. */
internalRoutes.post(
  '/politics/:entityId/wallet/credit',
  requireServiceRole(SERVICE_ROLES.politicsManage),
  validate(politicalEntityIdParams, 'params'),
  validate(movementBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reference, externalId, type } = req.body;
    const account = await ensurePoliticalAccount(req.params.entityId, currency ?? 'credits');
    res.status(201).json(
      await creditAccount(account.id, amount, { currency, type, reference, externalId, caller: requireService(req).clientId }),
    );
  }),
);

/** POST /politics/:entityId/wallet/debit — Treasury debit (rejected when balance insufficient). */
internalRoutes.post(
  '/politics/:entityId/wallet/debit',
  requireServiceRole(SERVICE_ROLES.politicsManage),
  validate(politicalEntityIdParams, 'params'),
  validate(movementBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reference, externalId, type } = req.body;
    const account = await ensurePoliticalAccount(req.params.entityId, currency ?? 'credits');
    res.status(201).json(
      await debitAccount(account.id, amount, { currency, type, reference, externalId, caller: requireService(req).clientId }),
    );
  }),
);

/** POST /politics/:entityId/mint — Create currency and credit the entity's treasury. */
internalRoutes.post(
  '/politics/:entityId/mint',
  requireServiceRole(SERVICE_ROLES.moneyIssue),
  validate(politicalEntityIdParams, 'params'),
  validate(mintBody),
  asyncHandler(async (req, res) => {
    const { amount, currency, reason } = req.body;
    res.status(201).json(
      await issueCurrency(req.params.entityId, amount, currency ?? 'credits', reason, requireService(req).clientId),
    );
  }),
);

/** POST /politics/:entityId/taxes/assess — Compute and book the entity's tax debts. */
internalRoutes.post(
  '/politics/:entityId/taxes/assess',
  requireServiceRole(SERVICE_ROLES.politicsManage),
  validate(politicalEntityIdParams, 'params'),
  validate(assessBody),
  asyncHandler(async (req, res) => {
    res.json(await assessTaxes(req.params.entityId, req.body.currency ?? 'credits'));
  }),
);

/** GET /politics/:entityId/settings — Tax rates and minting policy. */
internalRoutes.get(
  '/politics/:entityId/settings',
  requireServiceRole(SERVICE_ROLES.politicsRead),
  validate(politicalEntityIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await getPoliticalSettings(req.params.entityId));
  }),
);

/** PUT /politics/:entityId/settings — Update tax rates and minting policy. */
internalRoutes.put(
  '/politics/:entityId/settings',
  requireServiceRole(SERVICE_ROLES.politicsManage),
  validate(politicalEntityIdParams, 'params'),
  validate(politicalSettingsBody),
  asyncHandler(async (req, res) => {
    await getPoliticalSettings(req.params.entityId);
    res.json(await updatePoliticalSettings(req.params.entityId, req.body));
  }),
);

/** PUT /politics/:entityId/members/:playerId {role} — Set a member (and their role). */
internalRoutes.put(
  '/politics/:entityId/members/:playerId',
  requireServiceRole(SERVICE_ROLES.politicsManage),
  validate(politicalMemberParams, 'params'),
  validate(politicalMemberRoleBody),
  asyncHandler(async (req, res) => {
    res.json(
      await setPoliticalMember(req.params.entityId, req.params.playerId, req.body.role, req.body.holderType),
    );
  }),
);

/** DELETE /politics/:entityId/members/:playerId — Remove a member. */
internalRoutes.delete(
  '/politics/:entityId/members/:playerId',
  requireServiceRole(SERVICE_ROLES.politicsManage),
  validate(politicalMemberParams, 'params'),
  asyncHandler(async (req, res) => {
    await removePoliticalMember(req.params.entityId, req.params.playerId);
    res.status(204).send();
  }),
);

/** GET /politics/:entityId/members?limit=&offset= — Treasury members. */
internalRoutes.get(
  '/politics/:entityId/members',
  requireServiceRole(SERVICE_ROLES.politicsRead),
  validate(politicalEntityIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await listPoliticalMembers(req.params.entityId, Number(req.query.limit), Number(req.query.offset)));
  }),
);

// ── NPC taxes (game server) ──────────────────────────────────────────────────

/** GET /npcs/:npcId/taxes?limit=&offset= — NPC tax debts. */
internalRoutes.get(
  '/npcs/:npcId/taxes',
  requireServiceRole(SERVICE_ROLES.politicsRead),
  validate(npcIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(await getTaxDebts('npc', req.params.npcId, { limit: Number(req.query.limit), offset: Number(req.query.offset) }));
  }),
);

/** POST /npcs/:npcId/taxes/pay — Settle the NPC's affordable due taxes. */
internalRoutes.post(
  '/npcs/:npcId/taxes/pay',
  requireServiceRole(SERVICE_ROLES.politicsManage),
  validate(npcIdParams, 'params'),
  validate(payTaxesBody),
  asyncHandler(async (req, res) => {
    res.json(await payTaxDebts('npc', req.params.npcId, { currency: req.body.currency, entityId: req.body.entityId }));
  }),
);