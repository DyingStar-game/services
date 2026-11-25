# Atmosphere model settings and rasters

Data of the `weather-atmosphere` model (see `../../README.md`, section "The atmosphere model").

- `defaut.toml` — the model's generic choices, valid for every body (no body-specific number).
- `sandbox.toml` — what the shared data does not give for Sandbox: 11 layers aligned on the dust
  sea, the relief, the ground/dust colour map, the dust sea (500–3 500 m) and the lid, with the
  worldbuilding decision of 2026-11-25 merged in (lower planetary albedo accepted, real rock colour,
  air emissivity by the data's rule, strict dust-sea top).
- A body without its own file runs on `defaut.toml` alone: everything else is derived from the
  system data and `../climate/tarsis.json`.

Rasters (raw little-endian float32, 512 × 256, equirectangular: longitude −180 → 180 left to right,
latitude 90 → −90 top to bottom):

| File | Content | Origin |
|---|---|---|
| `sandbox_relief_512x256.f32` | ground altitude (m), cell averages of the game's relief, lightly smoothed | exported from the game's terrain with `export_relief.gd` (weather R&D, 2026-11-24) |
| `sandbox_ground_albedo_512x256.f32` | albedo of the ground colour (the corundum rock of tarsis_3), 0–1 | exported from the game's biome/rock colours with the same tool |

These two files are a manual export for now. Later they should be produced **automatically when the
terrain is published** (the terrain pipeline writes them next to the tiles), so that the weather
always runs on the relief and colours the players see.
