-- Legacy zone kinds are gone: geometry (scene paths, area centers) now lives in the
-- Inventory POI registry, and a zone either names a system or references a POI.
-- Rewrite every mission still carrying `scene`/`area` zones, keeping only `system` zones.
UPDATE "missions"
SET "zones" = (
  SELECT coalesce(
    jsonb_agg(z) FILTER (WHERE z->>'kind' in ('system', 'poi')),
    '[]'::jsonb
  )
  FROM jsonb_array_elements("missions"."zones") AS z
)
WHERE EXISTS (
  SELECT 1
  FROM jsonb_array_elements("missions"."zones") AS z
  WHERE z->>'kind' NOT IN ('system', 'poi')
);
