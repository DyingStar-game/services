/**
 * Corporation treasury routes (`/api/corporations`). Membership data is pushed by the
 * game server via the internal API; roles rank leader > treasurer > member.
 */
import { Router, type IRouter } from 'express';

import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { ensureCorporationAccount, getCorporationAccounts } from '../services/accounts.service.js';
import {
  donate,
  getCorporationReport,
  listCorporationMembers,
  requireCorporationMember,
  requireCorporationRole,
} from '../services/corporations.service.js';
import { listCorporationTransactions } from '../services/transactions.service.js';
import { corporationIdParams, donationBody, limitQuery, reportQuery } from './schemas.js';

/** Router mounted at `/api/corporations`. */
export const corporationsRoutes: IRouter = Router();

/** GET /:corporationId/wallet — Treasury accounts (member only). */
corporationsRoutes.get(
  '/:corporationId/wallet',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    await requireCorporationMember(corporationId, player.id);
    await ensureCorporationAccount(corporationId);
    res.json({ corporationId, accounts: await getCorporationAccounts(corporationId) });
  }),
);

/** GET /:corporationId/wallet/transactions?limit= — Treasury ledger (member only). */
corporationsRoutes.get(
  '/:corporationId/wallet/transactions',
  validate(corporationIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    await requireCorporationMember(corporationId, player.id);
    res.json(await listCorporationTransactions(corporationId, Number(req.query.limit)));
  }),
);

/** GET /:corporationId/members — Treasury staff (member only). */
corporationsRoutes.get(
  '/:corporationId/members',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    await requireCorporationMember(corporationId, player.id);
    res.json(await listCorporationMembers(corporationId));
  }),
);

/** GET /:corporationId/report?from=&to= — Financial report (leader/treasurer only). */
corporationsRoutes.get(
  '/:corporationId/report',
  validate(corporationIdParams, 'params'),
  validate(reportQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    await requireCorporationRole(corporationId, player.id, 'treasurer');
    const from = req.query.from ? new Date(req.query.from as string) : undefined;
    const to = req.query.to ? new Date(req.query.to as string) : undefined;
    res.json(await getCorporationReport(corporationId, from, to));
  }),
);

/** POST /:corporationId/donations — Donate credits from the member's wallet (member only). */
corporationsRoutes.post(
  '/:corporationId/donations',
  validate(corporationIdParams, 'params'),
  validate(donationBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    const result = await donate(player.id, corporationId, req.body.amount, req.body.memo);
    res.status(201).json(result);
  }),
);