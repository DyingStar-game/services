CREATE TABLE "political_activity" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "political_activity_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"entity_id" uuid NOT NULL,
	"actor_id" uuid,
	"type" text NOT NULL,
	"details" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "political_entities" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"type" text NOT NULL,
	"name" text NOT NULL,
	"description" text,
	"banner_url" text,
	"parent_id" uuid,
	"head_id" uuid,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "political_entities_name_unique" UNIQUE("name")
);
--> statement-breakpoint
CREATE TABLE "political_members" (
	"entity_id" uuid NOT NULL,
	"player_id" uuid NOT NULL,
	"office_id" integer NOT NULL,
	"joined_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "political_members_entity_id_player_id_pk" PRIMARY KEY("entity_id","player_id")
);
--> statement-breakpoint
CREATE TABLE "political_offices" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "political_offices_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"entity_id" uuid NOT NULL,
	"name" text NOT NULL,
	"priority" integer DEFAULT 0 NOT NULL,
	"permissions" text[] DEFAULT '{}'::text[] NOT NULL,
	"is_head" boolean DEFAULT false NOT NULL,
	"is_default" boolean DEFAULT false NOT NULL,
	CONSTRAINT "political_offices_name_unique" UNIQUE("entity_id","name")
);
--> statement-breakpoint
ALTER TABLE "political_activity" ADD CONSTRAINT "political_activity_entity_id_political_entities_id_fk" FOREIGN KEY ("entity_id") REFERENCES "public"."political_entities"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "political_activity" ADD CONSTRAINT "political_activity_actor_id_player_profiles_player_id_fk" FOREIGN KEY ("actor_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "political_entities" ADD CONSTRAINT "political_entities_parent_id_political_entities_id_fk" FOREIGN KEY ("parent_id") REFERENCES "public"."political_entities"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "political_entities" ADD CONSTRAINT "political_entities_head_id_player_profiles_player_id_fk" FOREIGN KEY ("head_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "political_members" ADD CONSTRAINT "political_members_entity_id_political_entities_id_fk" FOREIGN KEY ("entity_id") REFERENCES "public"."political_entities"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "political_members" ADD CONSTRAINT "political_members_player_id_player_profiles_player_id_fk" FOREIGN KEY ("player_id") REFERENCES "public"."player_profiles"("player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "political_members" ADD CONSTRAINT "political_members_office_id_political_offices_id_fk" FOREIGN KEY ("office_id") REFERENCES "public"."political_offices"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "political_offices" ADD CONSTRAINT "political_offices_entity_id_political_entities_id_fk" FOREIGN KEY ("entity_id") REFERENCES "public"."political_entities"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "political_activity_entity_created_idx" ON "political_activity" USING btree ("entity_id","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "political_entities_parent_idx" ON "political_entities" USING btree ("parent_id");--> statement-breakpoint
CREATE INDEX "political_entities_type_idx" ON "political_entities" USING btree ("type");--> statement-breakpoint
CREATE INDEX "political_members_player_idx" ON "political_members" USING btree ("player_id");