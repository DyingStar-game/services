/**
 * Pagination contract shared by every list endpoint of this service:
 * `?limit=&offset=` in, `{ items, total, limit, offset }` out.
 */
import { z } from 'zod';

/** Query parameters of the pagination contract (defaults applied when omitted). */
export const pageQuery = z.object({
  limit: z.coerce.number().int().min(1).max(100).default(20),
  offset: z.coerce.number().int().min(0).default(0),
});

/** Envelope returned by every paginated list. */
export interface Page<T> {
  items: T[];
  /** Total number of rows matching the filter, ignoring `limit`/`offset`. */
  total: number;
  limit: number;
  offset: number;
}

/** Builds the pagination envelope. */
export function page<T>(items: T[], total: number, limit: number, offset: number): Page<T> {
  return { items, total, limit, offset };
}

/** `limit`/`offset` extracted from a validated query (see `pageQuery`). */
export interface PageParams {
  limit: number;
  offset: number;
}
