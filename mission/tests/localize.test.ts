/** A mission's texts are served in the request's language when it carries them per language. */
import { describe, expect, it } from 'vitest';
import { localize } from '../src/middleware/localize.js';

describe('localize', () => {
  it('serves title and description in the language asked, falling back to the plain text', () => {
    const body = {
      mission: { title: 'Gold run', titleI18n: { en: 'Gold run', fr: "Course à l'or" }, description: null },
      objectives: [{ title: 'Deliver', titleI18n: { fr: 'Livrez' }, descriptionI18n: { fr: 'Au dépôt' } }],
      when: new Date(0),
    };
    const fr = localize(body, 'fr') as typeof body;
    expect(fr.mission.title).toBe("Course à l'or");
    expect(fr.objectives[0].title).toBe('Livrez');
    expect((fr.objectives[0] as Record<string, unknown>).description).toBe('Au dépôt');
    expect(fr.when).toBeInstanceOf(Date);
    const en = localize(body, 'en') as typeof body;
    expect(en.objectives[0].title).toBe('Deliver');
    expect(body.mission.title).toBe('Gold run');
  });
});
