/** The DataPad's tabs list the caller's missions by a set of statuses: one, or several comma-separated. */
import { describe, expect, it } from 'vitest';
import { assignmentListQuery } from '../src/routes/schemas.js';

describe('assignmentListQuery', () => {
  it('reads one status or several, and refuses an unknown one', () => {
    expect(assignmentListQuery.parse({ status: 'active' }).status).toEqual(['active']);
    expect(assignmentListQuery.parse({ status: 'completed, partially_completed' }).status).toEqual([
      'completed',
      'partially_completed',
    ]);
    expect(assignmentListQuery.parse({}).status).toBeUndefined();
    expect(assignmentListQuery.safeParse({ status: 'active,lost' }).success).toBe(false);
  });
});
