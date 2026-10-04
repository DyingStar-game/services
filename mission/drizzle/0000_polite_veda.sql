CREATE TABLE "mission_objectives" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"mission_id" uuid NOT NULL,
	"type" text NOT NULL,
	"title" text NOT NULL,
	"description" text,
	"target_quantity" integer DEFAULT 1 NOT NULL,
	"unit" text,
	"location_from" jsonb,
	"location_to" jsonb,
	"order" integer DEFAULT 0 NOT NULL,
	"status" text DEFAULT 'pending' NOT NULL,
	"current_progress" integer DEFAULT 0 NOT NULL,
	"params" jsonb,
	"payload" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "missions" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"title" text NOT NULL,
	"description" text,
	"kind" text DEFAULT 'dynamic' NOT NULL,
	"category" text DEFAULT 'generic' NOT NULL,
	"issuer_type" text DEFAULT 'system' NOT NULL,
	"issuer_id" uuid,
	"status" text DEFAULT 'available' NOT NULL,
	"rewards" jsonb,
	"prerequisites" jsonb NOT NULL,
	"visibility" text DEFAULT 'public' NOT NULL,
	"group_id" uuid,
	"group_claimable" boolean DEFAULT false NOT NULL,
	"is_event" boolean DEFAULT false NOT NULL,
	"zones" jsonb NOT NULL,
	"max_assignees" integer DEFAULT 1 NOT NULL,
	"escrow_status" text DEFAULT 'none' NOT NULL,
	"escrow_amount" integer,
	"escrow_currency" text,
	"escrow_payer_id" uuid,
	"escrow_external_id" text,
	"escrow_item_status" text DEFAULT 'none' NOT NULL,
	"escrow_item_hold_ids" jsonb,
	"script_id" text,
	"expires_at" timestamp with time zone,
	"created_by" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "mission_assignments" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"mission_id" uuid NOT NULL,
	"player_id" uuid NOT NULL,
	"holder_type" text DEFAULT 'player' NOT NULL,
	"status" text DEFAULT 'active' NOT NULL,
	"accepted_at" timestamp with time zone DEFAULT now() NOT NULL,
	"completed_at" timestamp with time zone,
	"reward_shares" jsonb,
	"reward_amount" integer,
	"reward_settled" boolean DEFAULT false NOT NULL,
	"reward_external_id" text NOT NULL,
	"reward_settled_at" timestamp with time zone,
	"reward_details" jsonb,
	"settled_components" text[] NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "mission_assignments_mission_player_unique" UNIQUE("mission_id","player_id"),
	CONSTRAINT "mission_assignments_reward_external_id_unique" UNIQUE("reward_external_id")
);
--> statement-breakpoint
ALTER TABLE "mission_objectives" ADD CONSTRAINT "mission_objectives_mission_id_missions_id_fk" FOREIGN KEY ("mission_id") REFERENCES "public"."missions"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "mission_assignments" ADD CONSTRAINT "mission_assignments_mission_id_missions_id_fk" FOREIGN KEY ("mission_id") REFERENCES "public"."missions"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "mission_objectives_mission_order_idx" ON "mission_objectives" USING btree ("mission_id","order");--> statement-breakpoint
CREATE UNIQUE INDEX "missions_script_unique" ON "missions" USING btree ("script_id");--> statement-breakpoint
CREATE UNIQUE INDEX "missions_escrow_external_id_unique" ON "missions" USING btree ("escrow_external_id");--> statement-breakpoint
CREATE INDEX "missions_visibility_status_idx" ON "missions" USING btree ("visibility","status");--> statement-breakpoint
CREATE INDEX "mission_assignments_player_idx" ON "mission_assignments" USING btree ("player_id","status");--> statement-breakpoint
CREATE INDEX "mission_assignments_mission_idx" ON "mission_assignments" USING btree ("mission_id","status");