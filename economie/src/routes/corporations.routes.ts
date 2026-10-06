/**
 * Corporation treasury routes (`/api/corporations`). Membership data is pushed by the
 * game server via the internal API; roles rank leader > treasurer > member and drive the
 * local salary tiers. Who may *operate* the treasury is decided by Social
 * (`economie:treasury:manage` through `POST /api/internal/authorize`).
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
  requireTreasuryPermission,
} from '../services/corporations.service.js';
import { getTaxDebts } from '../services/politics.service.js';
import { payTaxDebts } from '../services/taxation.service.js';
import { listCorporationTransactions } from '../services/transactions.service.js';
import {
  getSalaries,
  payPrime,
  removeMemberSalary,
  runPayroll,
  setMemberSalary,
  setRoleSalary,
} from '../services/payroll.service.js';
import {
  corporationIdParams,
  corporationMemberParams,
  corporationRoleParams,
  donationBody,
  limitQuery,
  memberSalaryBody,
  payTaxesBody,
  primeBody,
  reportQuery,
  roleSalaryBody,
} from './schemas.js';

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

/** GET /:corporationId/report?from=&to= — Financial report (`economie:treasury:manage`). */
corporationsRoutes.get(
  '/:corporationId/report',
  validate(corporationIdParams, 'params'),
  validate(reportQuery, 'query'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    await requireTreasuryPermission(corporationId, player.id);
    const from = req.query.from ? new Date(req.query.from as string) : undefined;
    const to = req.query.to ? new Date(req.query.to as string) : undefined;
    res.json(await getCorporationReport(corporationId, from, to));
  }),
);

/** GET /:corporationId/taxes — Tax debts of the corporation (member only). */
corporationsRoutes.get(
  '/:corporationId/taxes',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    await requireCorporationMember(corporationId, player.id);
    res.json(await getTaxDebts('corporation', corporationId));
  }),
);

/** POST /:corporationId/taxes/pay — Settle the corporation's affordable due taxes (`economie:treasury:manage`). */
corporationsRoutes.post(
  '/:corporationId/taxes/pay',
  validate(corporationIdParams, 'params'),
  validate(payTaxesBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    await requireTreasuryPermission(corporationId, player.id);
    res.json(await payTaxDebts('corporation', corporationId, { currency: req.body.currency, entityId: req.body.entityId }));
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

// ── Payroll (salaries & primes) — economie:treasury:manage only ──────────────────

/** GET /:corporationId/salaries — Role defaults and per-member overrides (`economie:treasury:manage`). */
corporationsRoutes.get(
  '/:corporationId/salaries',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    await requireTreasuryPermission(corporationId, player.id);
    res.json(await getSalaries(corporationId));
  }),
);

/** PUT /:corporationId/salaries/roles/:role — Set a role default salary (`economie:treasury:manage`). */
corporationsRoutes.put(
  '/:corporationId/salaries/roles/:role',
  validate(corporationRoleParams, 'params'),
  validate(roleSalaryBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    await requireTreasuryPermission(corporationId, player.id);
    res.json(await setRoleSalary(corporationId, req.params.role as never, req.body));
  }),
);

/** PUT /:corporationId/salaries/members/:playerId — Set a member override (`economie:treasury:manage`). */
corporationsRoutes.put(
  '/:corporationId/salaries/members/:playerId',
  validate(corporationMemberParams, 'params'),
  validate(memberSalaryBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId, playerId } = req.params;
    await requireTreasuryPermission(corporationId, player.id);
    res.json(await setMemberSalary(corporationId, playerId, req.body));
  }),
);

/** DELETE /:corporationId/salaries/members/:playerId — Drop an override (`economie:treasury:manage`). */
corporationsRoutes.delete(
  '/:corporationId/salaries/members/:playerId',
  validate(corporationMemberParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId, playerId } = req.params;
    await requireTreasuryPermission(corporationId, player.id);
    await removeMemberSalary(corporationId, playerId);
    res.status(204).send();
  }),
);

/** POST /:corporationId/payroll — Pay every member's salary now (`economie:treasury:manage`). */
corporationsRoutes.post(
  '/:corporationId/payroll',
  validate(corporationIdParams, 'params'),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId } = req.params;
    await requireTreasuryPermission(corporationId, player.id);
    res.json(await runPayroll(corporationId, (req.query.currency as string | undefined) ?? 'credits'));
  }),
);

/** POST /:corporationId/members/:playerId/prime — Pay a one-off prime (`economie:treasury:manage`). */
corporationsRoutes.post(
  '/:corporationId/members/:playerId/prime',
  validate(corporationMemberParams, 'params'),
  validate(primeBody),
  asyncHandler(async (req, res) => {
    const player = requirePlayer(req);
    const { corporationId, playerId } = req.params;
    await requireTreasuryPermission(corporationId, player.id);
    res.status(201).json(await payPrime(corporationId, playerId, req.body.amount, { currency: req.body.currency, memo: req.body.memo }));
  }),
);