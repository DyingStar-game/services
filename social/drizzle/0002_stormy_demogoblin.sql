CREATE TABLE "moderation_log" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "moderation_log_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"actor_id" uuid,
	"action" text NOT NULL,
	"target_player_id" uuid,
	"details" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "reports" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "reports_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"reporter_id" uuid,
	"target_type" text NOT NULL,
	"target_player_id" uuid,
	"target_guild_id" uuid,
	"reason" text NOT NULL,
	"message" text,
	"status" text DEFAULT 'open' NOT NULL,
	"escalation" text DEFAULT 'moderator' NOT NULL,
	"resolved_by" uuid,
	"resolution_note" text,
	"resolved_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "reputation_events" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "reputation_events_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"player_id" uuid NOT NULL,
	"delta" integer NOT NULL,
	"balance" integer NOT NULL,
	"source" text NOT NULL,
	"reason" text NOT NULL,
	"actor_id" uuid,
	"details" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "sanctions" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "sanctions_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"player_id" uuid NOT NULL,
	"type" text NOT NULL,
	"reason" text NOT NULL,
	"automatic" boolean DEFAULT false NOT NULL,
	"issued_by" uuid,
	"expires_at" timestamp with time zone,
	"revoked_at" timestamp with time zone,
	"revoked_by" uuid,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "reports" ADD CONSTRAINT "reports_reporter_id_player_profiles_player_id_fk" FOREIGN KEY ("reporter_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "reports" ADD CONSTRAINT "reports_target_player_id_player_profiles_player_id_fk" FOREIGN KEY ("target_player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "reports" ADD CONSTRAINT "reports_target_guild_id_guilds_id_fk" FOREIGN KEY ("target_guild_id") REFERENCES "public"."guilds"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "reputation_events" ADD CONSTRAINT "reputation_events_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "reputation_events" ADD CONSTRAINT "reputation_events_actor_id_player_profiles_player_id_fk" FOREIGN KEY ("actor_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "sanctions" ADD CONSTRAINT "sanctions_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "moderation_log_created_idx" ON "moderation_log" USING btree ("created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "reports_status_created_idx" ON "reports" USING btree ("status","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "reports_target_player_idx" ON "reports" USING btree ("target_player_id");--> statement-breakpoint
CREATE INDEX "reputation_events_player_created_idx" ON "reputation_events" USING btree ("player_id","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "sanctions_player_created_idx" ON "sanctions" USING btree ("player_id","created_at" DESC NULLS LAST);