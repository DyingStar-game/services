-- 0005_mission_invitations
--
-- A participant shares a mission already taken with their group or one player: joining opens an
-- assignment of the same mission (shared progress, reward split equally).
CREATE TABLE IF NOT EXISTS "mission_invitations" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"mission_id" uuid NOT NULL REFERENCES "missions"("id") ON DELETE cascade,
	"from_player_id" uuid NOT NULL,
	"to_player_id" uuid,
	"group_id" uuid,
	"status" text DEFAULT 'pending' NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "mission_invitations_to_player_idx" ON "mission_invitations" ("to_player_id","status");--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "mission_invitations_group_idx" ON "mission_invitations" ("group_id","status");--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "mission_invitations_mission_idx" ON "mission_invitations" ("mission_id");
