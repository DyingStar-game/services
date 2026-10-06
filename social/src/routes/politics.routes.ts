/**
 * Political entity routes (`/api/politics`, authenticated player).
 */
import { Router, type IRouter } from 'express';

import type { PoliticalEntityType } from '../db/schema/index.js';
import { asyncHandler } from '../lib/asyncHandler.js';
import { requirePlayer } from '../middleware/auth.js';
import { validate } from '../middleware/validate.js';
import { listPoliticalActivity } from '../services/politicalActivity.service.js';
import * as offices from '../services/politicalOffices.service.js';
import * as politics from '../services/politics.service.js';
import {
  createPoliticalEntityBody,
  limitQuery,
  politicalEntityIdParams,
  politicalEntityMemberParams,
  politicalEntityPatchBody,
  politicalMemberBody,
  politicalMemberOfficeBody,
  politicalOfficeBody,
  politicalOfficeParams,
  politicalOfficePatchBody,
  politicalParentBody,
  politicsListQuery,
  targetPlayerBody,
} from './schemas.js';

/** Router for political entities, their members and offices. */
export const politicsRoutes: IRouter = Router();

/** GET /?search=&type=&limit=&offset= — Public directory (`{ items, total, limit, offset }`). */
politicsRoutes.get(
  '/',
  validate(politicsListQuery, 'query'),
  asyncHandler(async (req, res) => {
    res.json(
      await politics.listPoliticalEntities(
        String(req.query.search ?? ''),
        req.query.type as PoliticalEntityType | undefined,
        Number(req.query.limit),
        Number(req.query.offset),
      ),
    );
  }),
);

/** POST / — Create a political entity; the caller becomes the head. */
politicsRoutes.post(
  '/',
  validate(createPoliticalEntityBody),
  asyncHandler(async (req, res) => {
    res.status(201).json(await politics.createPoliticalEntity(requirePlayer(req).id, req.body));
  }),
);

/** GET /:entityId — Public page (offices, members, presence, hierarchy). */
politicsRoutes.get(
  '/:entityId',
  validate(politicalEntityIdParams, 'params'),
  asyncHandler(async (req, res) => {
    res.json(await politics.getPoliticalEntityPage(req.params.entityId));
  }),
);

/** PATCH /:entityId — Edit the entity (manage_entity). */
politicsRoutes.patch(
  '/:entityId',
  validate(politicalEntityIdParams, 'params'),
  validate(politicalEntityPatchBody),
  asyncHandler(async (req, res) => {
    res.json(await politics.updatePoliticalEntity(req.params.entityId, requirePlayer(req).id, req.body));
  }),
);

/** DELETE /:entityId — Disband (head). */
politicsRoutes.delete(
  '/:entityId',
  validate(politicalEntityIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await politics.disbandPoliticalEntity(req.params.entityId, requirePlayer(req).id);
    res.status(204).send();
  }),
);

/** GET /:entityId/children — Direct lower-level entities (public, paginated). */
politicsRoutes.get(
  '/:entityId/children',
  validate(politicalEntityIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    await politics.requirePoliticalEntity(req.params.entityId);
    res.json(await politics.listPoliticalChildren(req.params.entityId, Number(req.query.limit), Number(req.query.offset)));
  }),
);

/** PUT /:entityId/parent — Attach to a higher-level entity, or detach (manage_hierarchy). */
politicsRoutes.put(
  '/:entityId/parent',
  validate(politicalEntityIdParams, 'params'),
  validate(politicalParentBody),
  asyncHandler(async (req, res) => {
    res.json(
      await politics.setPoliticalEntityParent(req.params.entityId, requirePlayer(req).id, req.body.parentId ?? null),
    );
  }),
);

/** POST /:entityId/transfer — Transfer the head office (head). */
politicsRoutes.post(
  '/:entityId/transfer',
  validate(politicalEntityIdParams, 'params'),
  validate(targetPlayerBody),
  asyncHandler(async (req, res) => {
    res.json(await politics.transferPoliticalHead(req.params.entityId, requirePlayer(req).id, req.body.playerId));
  }),
);

/** GET /:entityId/activity — Internal journal (members only). */
politicsRoutes.get(
  '/:entityId/activity',
  validate(politicalEntityIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    await politics.requirePoliticalMember(req.params.entityId, requirePlayer(req).id);
    res.json(await listPoliticalActivity(req.params.entityId, Number(req.query.limit), Number(req.query.offset)));
  }),
);

// ── Members ─────────────────────────────────────────────────────────────────

/** GET /:entityId/members — Members with office and presence (paginated). */
politicsRoutes.get(
  '/:entityId/members',
  validate(politicalEntityIdParams, 'params'),
  validate(limitQuery, 'query'),
  asyncHandler(async (req, res) => {
    await politics.requirePoliticalEntity(req.params.entityId);
    res.json(await politics.listPoliticalMembers(req.params.entityId, Number(req.query.limit), Number(req.query.offset)));
  }),
);

/** POST /:entityId/members — Appoint a member (manage_members). */
politicsRoutes.post(
  '/:entityId/members',
  validate(politicalEntityIdParams, 'params'),
  validate(politicalMemberBody),
  asyncHandler(async (req, res) => {
    res
      .status(201)
      .json(
        await politics.addPoliticalMember(
          req.params.entityId,
          requirePlayer(req).id,
          req.body.playerId,
          req.body.officeId,
        ),
      );
  }),
);

/** PATCH /:entityId/members/:playerId — Change a member's office (manage_members). */
politicsRoutes.patch(
  '/:entityId/members/:playerId',
  validate(politicalEntityMemberParams, 'params'),
  validate(politicalMemberOfficeBody),
  asyncHandler(async (req, res) => {
    res.json(
      await politics.setPoliticalMemberOffice(
        req.params.entityId,
        requirePlayer(req).id,
        req.params.playerId,
        req.body.officeId,
      ),
    );
  }),
);

/** DELETE /:entityId/members/:playerId — Leave (self) or remove (manage_members). */
politicsRoutes.delete(
  '/:entityId/members/:playerId',
  validate(politicalEntityMemberParams, 'params'),
  asyncHandler(async (req, res) => {
    await politics.removePoliticalMember(req.params.entityId, requirePlayer(req).id, req.params.playerId);
    res.status(204).send();
  }),
);

// ── Offices ─────────────────────────────────────────────────────────────────

/** GET /:entityId/offices — Offices, highest priority first. */
politicsRoutes.get(
  '/:entityId/offices',
  validate(politicalEntityIdParams, 'params'),
  asyncHandler(async (req, res) => {
    await politics.requirePoliticalEntity(req.params.entityId);
    res.json(await politics.listPoliticalOffices(req.params.entityId));
  }),
);

/** POST /:entityId/offices — Create an office (manage_offices). */
politicsRoutes.post(
  '/:entityId/offices',
  validate(politicalEntityIdParams, 'params'),
  validate(politicalOfficeBody),
  asyncHandler(async (req, res) => {
    res.status(201).json(await offices.createPoliticalOffice(req.params.entityId, requirePlayer(req).id, req.body));
  }),
);

/** PATCH /:entityId/offices/:officeId — Edit an office (manage_offices). */
politicsRoutes.patch(
  '/:entityId/offices/:officeId',
  validate(politicalOfficeParams, 'params'),
  validate(politicalOfficePatchBody),
  asyncHandler(async (req, res) => {
    res.json(
      await offices.updatePoliticalOffice(
        req.params.entityId,
        requirePlayer(req).id,
        Number(req.params.officeId),
        req.body,
      ),
    );
  }),
);

/** DELETE /:entityId/offices/:officeId — Delete an office (manage_offices). */
politicsRoutes.delete(
  '/:entityId/offices/:officeId',
  validate(politicalOfficeParams, 'params'),
  asyncHandler(async (req, res) => {
    await offices.deletePoliticalOffice(req.params.entityId, requirePlayer(req).id, Number(req.params.officeId));
    res.status(204).send();
  }),
);
