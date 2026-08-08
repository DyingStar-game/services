List of task a NPC can play (actions):

 - reception_players
 - depot_store_reception_to_temp_area
    reception_deposited_item (bool), if not null, there is a box on the reception deposit
    pnj_box_temp_zones (array {"zone_0: 1}), 1 = number of boxes in the temp zone

 - depot_store_item

    Moves boxes from the temp zone onto the warehouse shelves — the step after
    depot_store_reception_to_temp_area. Priority 94 (just below the reception
    action's 95): a fresh deposit on the counter always wins. The two actions
    use separate "box in hands" blackboard keys and each refuses to start while
    the other one's box is in hands, so an interrupted carry always resumes in
    the right flow.

    Triggers while AtWork whenever the depot's `pnj_box_temp_zones` lists a box
    that is replicated into view and not parented to a player (nobody carrying
    it). Before grabbing anything the NPC checks that some shelf has a free
    slot — a box grabbed with nowhere to put it down would stay in its hands
    forever. It then walks to the matching `pnj_zones.TempZone` place, picks
    the box up (grab confirmed by the box reparenting under the NPC), carries
    it to the NEAREST usable shelf, and drops it into the lowest free slot
    (drop confirmed by the box reparenting off the NPC; the slot is re-checked
    after the walk and re-picked if someone filled it meanwhile).

    ### Shelf contract

    Shelves replicate as objects: `object_type: "shelf"`, `parent_id` = depot
    uuid, `position`/`rotation` depot-local (radians, yaw in `rotation.y`),
    `scenename`, and ONE slot property:

    - `shelf_slots_occupied`: array of booleans, one per slot, `true` =
      occupied. Server-maintained: flipped when a box lands in / leaves a
      slot's detection area.

    The slot GEOMETRY is deliberately NOT replicated (it is static per scene
    and only the NPC needs it): it ships as JSON files exported from the
    Godot editor, loaded by the NPC service at startup from
    `SLOT_LAYOUTS_DIR` (default `config/slot_layouts`). The files mirror the
    Godot scene tree — the layout for
    `res://scenes/_universe/props/furniture/metal_shelf.tscn` lives at
    `config/slot_layouts/scenes/_universe/props/furniture/metal_shelf.json`
    (the relative path, `.json` -> `.tscn`, is the lookup key):

    ```json
    {
      "schema_version": 1,
      "scenename": "scenes/_universe/props/furniture/metal_shelf.tscn",
      "object_type": "shelf",
      "front": [0.0, 0.0, 1.0],
      "slots": [[-1.25, 0.35, 0.42], [-0.75, 0.35, 0.42], ...]
    }
    ```

    - `slots[i]` is the centre of slot `i`, LOCAL TO THE SCENE ROOT NODE, in
      metres, index-matched with the replicated `shelf_slots_occupied` —
      export them from the scene's slot markers, ordered bottom row first,
      left-to-right as seen when facing the front.
    - `front` is the access-face direction in the scene's local frame (the
      side the NPC approaches from; default +Z). Its length scales the
      standing distance: the NPC stands at `slot + front × stand_distance_m`,
      so `[2.5, 0, 0]` means "approach from +X, 2.5× the configured
      distance".
    - Depot-local point = `shelf.position + rotate_y(shelf.rotation.y) * local`
      (Godot yaw: `Vector3(1,0,0).rotated(UP, PI/2) == Vector3(0,0,-1)`).
    - A shelf whose scene has no layout file (or whose
      `shelf_slots_occupied` length mismatches the layout's slot count) is
      treated as "not usable yet" and silently skipped — the action stays
      inert-but-safe until the layout is exported.
    - The temp-zone shelf — same scene and `object_type` as the warehouse
      shelves — is NEVER a drop target: it is where the box comes from.
      Recognized by its `name` (`shelf_cargo_temp_zone` prefix, top-level or
      nested under `object_data`), and as a fallback geometrically: any shelf
      standing within 2.5 m (ground plane) of a `pnj_zones.TempZone` place
      (`target` point or rectangle centre) is treated as the temp rack even
      when no `name` replicates.
    - The NPC stands `stand_distance_m` metres (per-NPC config, default 1.0)
      in front of the target slot's column along `front`, at the shelf's
      floor height, and aims its head at the slot point before releasing
      the box.

    NOTE: the current
    `config/slot_layouts/scenes/_universe/props/furniture/metal_shelf.json`
    holds PLACEHOLDER positions (6 columns x 4 levels) — re-export the real
    marker positions from Godot.

    Assumption (server side): the depot removes a box's uuid from
    `pnj_box_temp_zones.zone_<n>` when the box leaves the temp deposit area,
    symmetric to how it adds it on entry. The NPC only reads that property.

 - depot_transfer_to_loading_dock
 - depot_transfert_to_reception
 - going_to_work
 - go_to_home
 - waiting
 - work_break
 - validate_mission
 - pay_players
 - load_into_vehicle
 - drive_to


