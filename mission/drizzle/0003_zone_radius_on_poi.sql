-- 0003_zone_radius_on_poi
--
-- `missions.zones[].radiusM` is gone: a mission zone is exactly `{kind, poiId}` and
-- inherits the POI's radius wholesale (the POI is the only geometric source). Leaving a
-- stale `radiusM` in the jsonb would silently widen the zone of a mission whose radius had
-- been tightened against the POI, so the residual values are stripped here.
update "missions"
set "zones" = (
  select jsonb_agg(case when z->>'kind' = 'poi' then z - 'radiusM' else z end)
  from jsonb_array_elements("missions"."zones") as z
)
where "zones" is not null
  and exists (
    select 1 from jsonb_array_elements("missions"."zones") as z
    where z->>'kind' = 'poi' and z ? 'radiusM'
  );
