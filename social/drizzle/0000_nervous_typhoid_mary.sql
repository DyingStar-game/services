CREATE TABLE "player_profiles" (
	"player_id" uuid PRIMARY KEY NOT NULL,
	"entity_type" text DEFAULT 'player' NOT NULL,
	"display_name" text NOT NULL,
	"avatar_url" text,
	"faction" text,
	"biography" text,
	"role" text,
	"level" integer DEFAULT 0 NOT NULL,
	"reputation" integer DEFAULT 0 NOT NULL,
	"playtime_seconds" bigint DEFAULT 0 NOT NULL,
	"rp_sheet" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "player_profiles_display_name_unique" UNIQUE("display_name")
);
--> statement-breakpoint
CREATE TABLE "player_presence" (
	"player_id" uuid PRIMARY KEY NOT NULL,
	"status" text DEFAULT 'offline' NOT NULL,
	"location" jsonb,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "friendships" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "friendships_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"requester_id" uuid NOT NULL,
	"addressee_id" uuid NOT NULL,
	"status" text DEFAULT 'pending' NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "friendships_pair_unique" UNIQUE("requester_id","addressee_id"),
	CONSTRAINT "friendships_not_self" CHECK ("friendships"."requester_id" <> "friendships"."addressee_id")
);
--> statement-breakpoint
CREATE TABLE "player_blocks" (
	"blocker_id" uuid NOT NULL,
	"blocked_id" uuid NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "player_blocks_blocker_id_blocked_id_pk" PRIMARY KEY("blocker_id","blocked_id")
);
--> statement-breakpoint
CREATE TABLE "player_encounters" (
	"player_id" uuid NOT NULL,
	"other_id" uuid NOT NULL,
	"count" integer DEFAULT 1 NOT NULL,
	"last_met_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "player_encounters_player_id_other_id_pk" PRIMARY KEY("player_id","other_id")
);
--> statement-breakpoint
CREATE TABLE "player_activity" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "player_activity_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"player_id" uuid NOT NULL,
	"type" text NOT NULL,
	"details" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "corporation_activity" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "corporation_activity_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"corporation_id" uuid NOT NULL,
	"actor_id" uuid,
	"type" text NOT NULL,
	"details" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "corporation_join_requests" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "corporation_join_requests_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"corporation_id" uuid NOT NULL,
	"player_id" uuid NOT NULL,
	"kind" text NOT NULL,
	"message" text,
	"created_by" uuid,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "corporation_join_requests_pair_unique" UNIQUE("corporation_id","player_id")
);
--> statement-breakpoint
CREATE TABLE "corporation_members" (
	"corporation_id" uuid NOT NULL,
	"player_id" uuid NOT NULL,
	"rank_id" integer NOT NULL,
	"joined_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "corporation_members_corporation_id_player_id_pk" PRIMARY KEY("corporation_id","player_id"),
	CONSTRAINT "corporation_members_player_unique" UNIQUE("player_id")
);
--> statement-breakpoint
CREATE TABLE "corporation_ranks" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "corporation_ranks_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"corporation_id" uuid NOT NULL,
	"name" text NOT NULL,
	"priority" integer DEFAULT 0 NOT NULL,
	"permissions" text[] DEFAULT '{}'::text[] NOT NULL,
	"is_ceo" boolean DEFAULT false NOT NULL,
	"is_default" boolean DEFAULT false NOT NULL,
	CONSTRAINT "corporation_ranks_name_unique" UNIQUE("corporation_id","name")
);
--> statement-breakpoint
CREATE TABLE "corporations" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"name" text NOT NULL,
	"ticker" text NOT NULL,
	"logo_url" text,
	"description" text,
	"recruitment" text DEFAULT 'apply' NOT NULL,
	"ceo_id" uuid NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "corporations_name_unique" UNIQUE("name"),
	CONSTRAINT "corporations_ticker_unique" UNIQUE("ticker")
);
--> statement-breakpoint
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
	"target_corporation_id" uuid,
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
ALTER TABLE "player_presence" ADD CONSTRAINT "player_presence_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "friendships" ADD CONSTRAINT "friendships_requester_id_player_profiles_player_id_fk" FOREIGN KEY ("requester_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "friendships" ADD CONSTRAINT "friendships_addressee_id_player_profiles_player_id_fk" FOREIGN KEY ("addressee_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "player_blocks" ADD CONSTRAINT "player_blocks_blocker_id_player_profiles_player_id_fk" FOREIGN KEY ("blocker_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "player_blocks" ADD CONSTRAINT "player_blocks_blocked_id_player_profiles_player_id_fk" FOREIGN KEY ("blocked_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "player_encounters" ADD CONSTRAINT "player_encounters_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "player_encounters" ADD CONSTRAINT "player_encounters_other_id_player_profiles_player_id_fk" FOREIGN KEY ("other_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "player_activity" ADD CONSTRAINT "player_activity_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "corporation_activity" ADD CONSTRAINT "corporation_activity_corporation_id_corporations_id_fk" FOREIGN KEY ("corporation_id") REFERENCES "public"."corporations"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "corporation_activity" ADD CONSTRAINT "corporation_activity_actor_id_player_profiles_player_id_fk" FOREIGN KEY ("actor_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "corporation_join_requests" ADD CONSTRAINT "corporation_join_requests_corporation_id_corporations_id_fk" FOREIGN KEY ("corporation_id") REFERENCES "public"."corporations"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "corporation_join_requests" ADD CONSTRAINT "corporation_join_requests_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "corporation_join_requests" ADD CONSTRAINT "corporation_join_requests_created_by_player_profiles_player_id_fk" FOREIGN KEY ("created_by") REFERENCES "public"."player_profiles"("player_id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "corporation_members" ADD CONSTRAINT "corporation_members_corporation_id_corporations_id_fk" FOREIGN KEY ("corporation_id") REFERENCES "public"."corporations"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "corporation_members" ADD CONSTRAINT "corporation_members_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "corporation_members" ADD CONSTRAINT "corporation_members_rank_id_corporation_ranks_id_fk" FOREIGN KEY ("rank_id") REFERENCES "public"."corporation_ranks"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "corporation_ranks" ADD CONSTRAINT "corporation_ranks_corporation_id_corporations_id_fk" FOREIGN KEY ("corporation_id") REFERENCES "public"."corporations"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "corporations" ADD CONSTRAINT "corporations_ceo_id_player_profiles_player_id_fk" FOREIGN KEY ("ceo_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "reports" ADD CONSTRAINT "reports_reporter_id_player_profiles_player_id_fk" FOREIGN KEY ("reporter_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "reports" ADD CONSTRAINT "reports_target_player_id_player_profiles_player_id_fk" FOREIGN KEY ("target_player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "reports" ADD CONSTRAINT "reports_target_corporation_id_corporations_id_fk" FOREIGN KEY ("target_corporation_id") REFERENCES "public"."corporations"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "reputation_events" ADD CONSTRAINT "reputation_events_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "reputation_events" ADD CONSTRAINT "reputation_events_actor_id_player_profiles_player_id_fk" FOREIGN KEY ("actor_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "sanctions" ADD CONSTRAINT "sanctions_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "player_activity_player_created_idx" ON "player_activity" USING btree ("player_id","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "corporation_activity_corporation_created_idx" ON "corporation_activity" USING btree ("corporation_id","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "moderation_log_created_idx" ON "moderation_log" USING btree ("created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "reports_status_created_idx" ON "reports" USING btree ("status","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "reports_target_player_idx" ON "reports" USING btree ("target_player_id");--> statement-breakpoint
CREATE INDEX "reputation_events_player_created_idx" ON "reputation_events" USING btree ("player_id","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "sanctions_player_created_idx" ON "sanctions" USING btree ("player_id","created_at" DESC NULLS LAST);