CREATE TABLE "guild_activity" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "guild_activity_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"guild_id" uuid NOT NULL,
	"actor_id" uuid,
	"type" text NOT NULL,
	"details" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "guild_join_requests" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "guild_join_requests_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"guild_id" uuid NOT NULL,
	"player_id" uuid NOT NULL,
	"kind" text NOT NULL,
	"message" text,
	"created_by" uuid,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "guild_join_requests_pair_unique" UNIQUE("guild_id","player_id")
);
--> statement-breakpoint
CREATE TABLE "guild_members" (
	"guild_id" uuid NOT NULL,
	"player_id" uuid NOT NULL,
	"rank_id" integer NOT NULL,
	"joined_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "guild_members_guild_id_player_id_pk" PRIMARY KEY("guild_id","player_id"),
	CONSTRAINT "guild_members_player_unique" UNIQUE("player_id")
);
--> statement-breakpoint
CREATE TABLE "guild_ranks" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "guild_ranks_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"guild_id" uuid NOT NULL,
	"name" text NOT NULL,
	"priority" integer DEFAULT 0 NOT NULL,
	"permissions" text[] DEFAULT '{}'::text[] NOT NULL,
	"is_leader" boolean DEFAULT false NOT NULL,
	"is_default" boolean DEFAULT false NOT NULL,
	CONSTRAINT "guild_ranks_name_unique" UNIQUE("guild_id","name")
);
--> statement-breakpoint
CREATE TABLE "guilds" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"name" text NOT NULL,
	"tag" text NOT NULL,
	"logo_url" text,
	"description" text,
	"recruitment" text DEFAULT 'apply' NOT NULL,
	"owner_id" uuid NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "guilds_name_unique" UNIQUE("name"),
	CONSTRAINT "guilds_tag_unique" UNIQUE("tag")
);
--> statement-breakpoint
ALTER TABLE "guild_activity" ADD CONSTRAINT "guild_activity_guild_id_guilds_id_fk" FOREIGN KEY ("guild_id") REFERENCES "public"."guilds"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "guild_activity" ADD CONSTRAINT "guild_activity_actor_id_player_profiles_player_id_fk" FOREIGN KEY ("actor_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "guild_join_requests" ADD CONSTRAINT "guild_join_requests_guild_id_guilds_id_fk" FOREIGN KEY ("guild_id") REFERENCES "public"."guilds"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "guild_join_requests" ADD CONSTRAINT "guild_join_requests_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "guild_join_requests" ADD CONSTRAINT "guild_join_requests_created_by_player_profiles_player_id_fk" FOREIGN KEY ("created_by") REFERENCES "public"."player_profiles"("player_id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "guild_members" ADD CONSTRAINT "guild_members_guild_id_guilds_id_fk" FOREIGN KEY ("guild_id") REFERENCES "public"."guilds"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "guild_members" ADD CONSTRAINT "guild_members_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "guild_members" ADD CONSTRAINT "guild_members_rank_id_guild_ranks_id_fk" FOREIGN KEY ("rank_id") REFERENCES "public"."guild_ranks"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "guild_ranks" ADD CONSTRAINT "guild_ranks_guild_id_guilds_id_fk" FOREIGN KEY ("guild_id") REFERENCES "public"."guilds"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "guilds" ADD CONSTRAINT "guilds_owner_id_player_profiles_player_id_fk" FOREIGN KEY ("owner_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "guild_activity_guild_created_idx" ON "guild_activity" USING btree ("guild_id","created_at" DESC NULLS LAST);