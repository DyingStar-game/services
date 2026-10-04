/**
 * Political office management (requires `manage_offices`). The head office is immutable except for
 * its name; the default office cannot be deleted without promoting another one first.
 */
import { and, eq } from 'drizzle-orm';
import { t } from '../i18n/index.js';

import { db } from '../db/connection.js';
import { politicalMembers, politicalOffices, type PoliticalOffice, type PoliticalPermission } from '../db/schema/index.js';
import { conflict, forbidden, notFound } from '../lib/httpError.js';
import { recordPoliticalActivity } from './politicalActivity.service.js';
import { getDefaultPoliticalOffice, requirePoliticalPermission } from './politics.service.js';

/** Office fields. */
export interface PoliticalOfficeInput {
  name: string;
  priority: number;
  permissions: PoliticalPermission[];
  isDefault?: boolean;
}

function isUniqueViolation(err: unknown): boolean {
  const pgError = (err as { cause?: unknown })?.cause ?? err;
  return (pgError as { code?: string } | undefined)?.code === '23505';
}

async function requireOffice(entityId: string, officeId: number): Promise<PoliticalOffice> {
  const [office] = await db
    .select()
    .from(politicalOffices)
    .where(and(eq(politicalOffices.id, officeId), eq(politicalOffices.entityId, entityId)))
    .limit(1);
  if (!office) throw notFound(t('not_found.office', { id: officeId }));
  return office;
}

/**
 * Creates an office below the actor's own office.
 * @param entityId - Entity id.
 * @param actorId - Acting member.
 * @param input - Office fields.
 * @returns Created office.
 */
export async function createPoliticalOffice(
  entityId: string,
  actorId: string,
  input: PoliticalOfficeInput,
): Promise<PoliticalOffice> {
  const actor = await requirePoliticalPermission(entityId, actorId, 'manage_offices');
  if (!actor.office.isHead && input.priority >= actor.office.priority) {
    throw forbidden(t('forbidden.office_priority'));
  }
  try {
    return await db.transaction(async (tx) => {
      if (input.isDefault) {
        await tx.update(politicalOffices).set({ isDefault: false }).where(eq(politicalOffices.entityId, entityId));
      }
      const [office] = await tx
        .insert(politicalOffices)
        .values({ entityId, ...input, isDefault: input.isDefault ?? false })
        .returning();
      await recordPoliticalActivity(entityId, actorId, 'office_created', { office: office.name }, tx);
      return office;
    });
  } catch (err) {
    if (isUniqueViolation(err)) throw conflict(t('conflict.office_exists', { name: input.name }));
    throw err;
  }
}

/**
 * Updates an office. The head office only accepts a name change; the actor must outrank the office.
 * @param entityId - Entity id.
 * @param actorId - Acting member.
 * @param officeId - Office id.
 * @param patch - Fields to change.
 * @returns Updated office.
 */
export async function updatePoliticalOffice(
  entityId: string,
  actorId: string,
  officeId: number,
  patch: Partial<PoliticalOfficeInput>,
): Promise<PoliticalOffice> {
  const actor = await requirePoliticalPermission(entityId, actorId, 'manage_offices');
  const office = await requireOffice(entityId, officeId);
  if (office.isHead) {
    if (patch.priority !== undefined || patch.permissions !== undefined || patch.isDefault) {
      throw forbidden(t('forbidden.head_office_name'));
    }
  } else {
    if (!actor.office.isHead && office.priority >= actor.office.priority) {
      throw forbidden(t('forbidden.edit_office_outrank'));
    }
    if (!actor.office.isHead && patch.priority !== undefined && patch.priority >= actor.office.priority) {
      throw forbidden(t('forbidden.office_priority'));
    }
  }
  if (patch.isDefault === false && office.isDefault) {
    throw conflict(t('conflict.default_office'));
  }
  try {
    return await db.transaction(async (tx) => {
      if (patch.isDefault) {
        await tx.update(politicalOffices).set({ isDefault: false }).where(eq(politicalOffices.entityId, entityId));
      }
      const [updated] = await tx.update(politicalOffices).set(patch).where(eq(politicalOffices.id, officeId)).returning();
      await recordPoliticalActivity(entityId, actorId, 'office_updated', { office: updated.name, fields: Object.keys(patch) }, tx);
      return updated;
    });
  } catch (err) {
    if (isUniqueViolation(err)) throw conflict(t('conflict.office_exists', { name: patch.name ?? '' }));
    throw err;
  }
}

/**
 * Deletes an office; its members are moved to the default office. Head and default offices cannot be deleted.
 * @param entityId - Entity id.
 * @param actorId - Acting member.
 * @param officeId - Office id.
 */
export async function deletePoliticalOffice(entityId: string, actorId: string, officeId: number): Promise<void> {
  const actor = await requirePoliticalPermission(entityId, actorId, 'manage_offices');
  const office = await requireOffice(entityId, officeId);
  if (office.isHead) throw forbidden(t('forbidden.head_office_undeletable'));
  if (office.isDefault) throw forbidden(t('forbidden.default_office_undeletable'));
  if (!actor.office.isHead && office.priority >= actor.office.priority) {
    throw forbidden(t('forbidden.delete_office_outrank'));
  }
  const defaultOffice = await getDefaultPoliticalOffice(entityId);
  await db.transaction(async (tx) => {
    await tx.update(politicalMembers).set({ officeId: defaultOffice.id }).where(eq(politicalMembers.officeId, officeId));
    await tx.delete(politicalOffices).where(eq(politicalOffices.id, officeId));
    await recordPoliticalActivity(entityId, actorId, 'office_deleted', { office: office.name }, tx);
  });
}
