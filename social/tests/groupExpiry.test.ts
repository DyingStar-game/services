/**
 * Idle groups: which ones the expiry pass disbands. A group is a squad to share missions; one
 * nobody came back to must not hold its members for ever ("already in a group").
 */
import { describe, expect, it, vi } from 'vitest';

import type { GroupActivity } from '../src/services/groups.service.js';

// The service opens its database pool when loaded; the rule under test is pure and never uses it.
vi.mock('../src/db/connection.js', () => ({ db: {} }));
const { idleGroupIds } = await import('../src/services/groups.service.js');

const HOUR = 3_600_000;
const NOW = Date.UTC(2026, 10, 28, 12);
const ago = (hours: number): Date => new Date(NOW - hours * HOUR);

function group(id: string, editedHoursAgo: number, members: GroupActivity['members']): GroupActivity {
  return { id, updatedAt: ago(editedHoursAgo), members };
}

describe('idleGroupIds', () => {
  it('disbands a group whose members were all last seen before the delay', () => {
    const activity = [
      group('gone', 200, [
        { joinedAt: ago(200), online: false, lastSeenAt: ago(30) },
        { joinedAt: ago(150), online: false, lastSeenAt: ago(48) },
      ]),
    ];
    expect(idleGroupIds(activity, NOW, 24 * HOUR)).toEqual(['gone']);
  });

  it('keeps a group with a member online, however long ago their presence last changed', () => {
    const activity = [group('playing', 200, [{ joinedAt: ago(200), online: true, lastSeenAt: ago(40) }])];
    expect(idleGroupIds(activity, NOW, 24 * HOUR)).toEqual([]);
  });

  it('keeps a group somebody was seen in, joined or edited within the delay', () => {
    const activity = [
      group('seen', 200, [{ joinedAt: ago(200), online: false, lastSeenAt: ago(2) }]),
      group('joined', 200, [
        { joinedAt: ago(200), online: false, lastSeenAt: ago(90) },
        { joinedAt: ago(3), online: false, lastSeenAt: null },
      ]),
      group('edited', 1, [{ joinedAt: ago(200), online: false, lastSeenAt: ago(90) }]),
    ];
    expect(idleGroupIds(activity, NOW, 24 * HOUR)).toEqual([]);
  });

  it('counts a member never seen from the day they joined', () => {
    const activity = [group('never', 100, [{ joinedAt: ago(100), online: false, lastSeenAt: null }])];
    expect(idleGroupIds(activity, NOW, 24 * HOUR)).toEqual(['never']);
  });
});
