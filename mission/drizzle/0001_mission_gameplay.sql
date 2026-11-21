ALTER TABLE "mission_objectives" ADD COLUMN "weight" integer DEFAULT 1 NOT NULL;--> statement-breakpoint
ALTER TABLE "mission_objectives" ADD COLUMN "is_bonus" boolean DEFAULT false NOT NULL;--> statement-breakpoint
ALTER TABLE "mission_objectives" ADD COLUMN "bonus_reward" jsonb;--> statement-breakpoint
ALTER TABLE "missions" ADD COLUMN "auto_complete" boolean DEFAULT true NOT NULL;--> statement-breakpoint
ALTER TABLE "missions" ADD COLUMN "allow_partial" boolean DEFAULT false NOT NULL;--> statement-breakpoint
ALTER TABLE "missions" ADD COLUMN "requires_mission_ids" uuid[] DEFAULT '{}' NOT NULL;--> statement-breakpoint
ALTER TABLE "missions" ADD COLUMN "assignment_deadline_hours" integer;--> statement-breakpoint
ALTER TABLE "mission_assignments" ADD COLUMN "deadline_at" timestamp with time zone;--> statement-breakpoint
ALTER TABLE "mission_assignments" ADD COLUMN "deadline_notified_at" timestamp with time zone;--> statement-breakpoint
ALTER TABLE "mission_assignments" ADD COLUMN "completion_ratio" integer DEFAULT 0 NOT NULL;