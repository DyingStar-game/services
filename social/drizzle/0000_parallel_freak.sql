CREATE TABLE "player_profiles" (
	"player_id" uuid PRIMARY KEY NOT NULL,
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
ALTER TABLE "player_presence" ADD CONSTRAINT "player_presence_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "friendships" ADD CONSTRAINT "friendships_requester_id_player_profiles_player_id_fk" FOREIGN KEY ("requester_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "friendships" ADD CONSTRAINT "friendships_addressee_id_player_profiles_player_id_fk" FOREIGN KEY ("addressee_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "player_blocks" ADD CONSTRAINT "player_blocks_blocker_id_player_profiles_player_id_fk" FOREIGN KEY ("blocker_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "player_blocks" ADD CONSTRAINT "player_blocks_blocked_id_player_profiles_player_id_fk" FOREIGN KEY ("blocked_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "player_encounters" ADD CONSTRAINT "player_encounters_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "player_encounters" ADD CONSTRAINT "player_encounters_other_id_player_profiles_player_id_fk" FOREIGN KEY ("other_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "player_activity" ADD CONSTRAINT "player_activity_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "player_activity_player_created_idx" ON "player_activity" USING btree ("player_id","created_at" DESC NULLS LAST);