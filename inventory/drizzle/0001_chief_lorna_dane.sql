CREATE TABLE "inventory_poi_shares" (
	"poi_id" uuid NOT NULL,
	"grantee_type" text NOT NULL,
	"grantee_id" uuid NOT NULL,
	"shared_by" uuid,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "inventory_poi_shares_poi_id_grantee_type_grantee_id_pk" PRIMARY KEY("poi_id","grantee_type","grantee_id"),
	CONSTRAINT "inventory_poi_shares_grantee_type_valid" CHECK ("inventory_poi_shares"."grantee_type" in ('player','npc','corporation','political'))
);
--> statement-breakpoint
CREATE TABLE "inventory_pois" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"owner_type" text NOT NULL,
	"owner_id" uuid NOT NULL,
	"name" text NOT NULL,
	"description" text,
	"system" text,
	"scene" text,
	"parent_id" uuid,
	"x" double precision NOT NULL,
	"y" double precision NOT NULL,
	"z" double precision NOT NULL,
	"radius_m" double precision,
	"visibility" text DEFAULT 'private' NOT NULL,
	"metadata" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "inventory_pois_owner_type_valid" CHECK ("inventory_pois"."owner_type" in ('player','npc','corporation','political','system')),
	CONSTRAINT "inventory_pois_radius_valid" CHECK ("inventory_pois"."radius_m" is null or "inventory_pois"."radius_m" >= 0),
	CONSTRAINT "inventory_pois_visibility_valid" CHECK ("inventory_pois"."visibility" in ('private','public')),
	CONSTRAINT "inventory_pois_name_not_empty" CHECK (length(btrim("inventory_pois"."name")) > 0)
);
--> statement-breakpoint
ALTER TABLE "inventory_poi_shares" ADD CONSTRAINT "inventory_poi_shares_poi_id_inventory_pois_id_fk" FOREIGN KEY ("poi_id") REFERENCES "public"."inventory_pois"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "inventory_poi_shares_grantee_idx" ON "inventory_poi_shares" USING btree ("grantee_type","grantee_id");--> statement-breakpoint
CREATE INDEX "inventory_pois_owner_idx" ON "inventory_pois" USING btree ("owner_type","owner_id");--> statement-breakpoint
CREATE INDEX "inventory_pois_system_idx" ON "inventory_pois" USING btree ("system");--> statement-breakpoint
CREATE INDEX "inventory_pois_scene_idx" ON "inventory_pois" USING btree ("scene");--> statement-breakpoint
CREATE INDEX "inventory_pois_visibility_idx" ON "inventory_pois" USING btree ("visibility");