-- 0007_mission_i18n
--
-- A mission's and an objective's title and description per language ({"en": "...", "fr": "..."}),
-- served by Accept-Language (localizeResponse). The plain columns stay the default text.
ALTER TABLE "missions" ADD COLUMN IF NOT EXISTS "title_i18n" jsonb;
--> statement-breakpoint
ALTER TABLE "missions" ADD COLUMN IF NOT EXISTS "description_i18n" jsonb;
--> statement-breakpoint
ALTER TABLE "mission_objectives" ADD COLUMN IF NOT EXISTS "title_i18n" jsonb;
--> statement-breakpoint
ALTER TABLE "mission_objectives" ADD COLUMN IF NOT EXISTS "description_i18n" jsonb;
