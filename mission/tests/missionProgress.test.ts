/** A list row's progress: the step under way, how far it is, and where the mission goes. */
import { describe, expect, it } from 'vitest';

import type { MissionObjective } from '../src/db/schema/index.js';
import { summarizeProgress } from '../src/lib/progress.js';

/** An objective with only the fields that matter here; the rest is filler. */
function objective(patch: Partial<MissionObjective>): MissionObjective {
  return {
    id: 'o',
    missionId: 'm',
    type: 'transport',
    title: 'Step',
    description: null,
    titleI18n: null,
    descriptionI18n: null,
    targetQuantity: 1,
    unit: null,
    locationFrom: null,
    locationTo: null,
    order: 0,
    weight: 1,
    isBonus: false,
    bonusReward: null,
    status: 'pending',
    currentProgress: 0,
    params: null,
    payload: null,
    state: null,
    createdAt: new Date(0),
    updatedAt: new Date(0),
    ...patch,
  } as MissionObjective;
}

describe('summarizeProgress', () => {
  it('names the first required step not completed, with its measure', () => {
    const progress = summarizeProgress([
      objective({ id: 'a', status: 'completed' }),
      objective({ id: 'b', status: 'in_progress', currentProgress: 30, targetQuantity: 90, unit: 'kg' }),
      objective({ id: 'c' }),
    ]);
    expect(progress.step).toBe(2);
    expect(progress.steps).toBe(3);
    expect(progress.done).toBe(1);
    expect(progress.current?.id).toBe('b');
    expect(progress.current?.currentProgress).toBe(30);
    expect(progress.current?.targetQuantity).toBe(90);
    expect(progress.current?.unit).toBe('kg');
  });

  it('leaves bonus objectives out of the steps', () => {
    const progress = summarizeProgress([
      objective({ id: 'bonus', isBonus: true }),
      objective({ id: 'a', status: 'completed' }),
    ]);
    expect(progress.steps).toBe(1);
    expect(progress.step).toBe(1);
    expect(progress.current).toBeNull();
  });

  it('reads the route from the first step naming both ends', () => {
    const progress = summarizeProgress([
      objective({ id: 'a', locationTo: { village: 'F0001' } }),
      objective({ id: 'b', locationFrom: { village: 'M0001' }, locationTo: { village: 'F0001' } }),
    ]);
    expect(progress.from).toEqual({ village: 'M0001' });
    expect(progress.to).toEqual({ village: 'F0001' });
  });

  it('says nothing is under way for a mission without objectives', () => {
    const progress = summarizeProgress([]);
    expect(progress).toMatchObject({ step: 0, steps: 0, done: 0, current: null, from: null, to: null });
  });
});
