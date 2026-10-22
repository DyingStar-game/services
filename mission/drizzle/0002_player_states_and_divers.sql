CREATE TABLE "mission_player_states" (
	"player_id" uuid PRIMARY KEY NOT NULL,
	"place" jsonb,
	"on_foot" boolean DEFAULT true NOT NULL,
	"passenger" boolean DEFAULT false NOT NULL,
	"carried" jsonb DEFAULT '{}'::jsonb NOT NULL,
	"received_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "missions" ALTER COLUMN "category" SET DEFAULT 'divers';--> statement-breakpoint
ALTER TABLE "mission_objectives" ADD COLUMN "state" jsonb;