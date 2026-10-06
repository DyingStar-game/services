CREATE TABLE "inventory_good_types" (
	"good_type" text PRIMARY KEY NOT NULL,
	"kind" text NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "inventory_instances" (
	"id" uuid PRIMARY KEY NOT NULL,
	"good_type" text NOT NULL,
	"holder_type" text NOT NULL,
	"holder_id" uuid NOT NULL,
	"status" text DEFAULT 'stored' NOT NULL,
	"metadata" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "inventory_stacks" (
	"holder_type" text NOT NULL,
	"holder_id" uuid NOT NULL,
	"good_type" text NOT NULL,
	"quantity" bigint DEFAULT 0 NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "inventory_stacks_holder_type_holder_id_good_type_pk" PRIMARY KEY("holder_type","holder_id","good_type"),
	CONSTRAINT "inventory_stacks_quantity_positive" CHECK ("inventory_stacks"."quantity" >= 0),
	CONSTRAINT "inventory_stacks_holder_type_valid" CHECK ("inventory_stacks"."holder_type" in ('player','npc','corporation','system'))
);
--> statement-breakpoint
CREATE TABLE "inventory_holds" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"holder_type" text NOT NULL,
	"holder_id" uuid NOT NULL,
	"kind" text NOT NULL,
	"good_type" text NOT NULL,
	"quantity" bigint NOT NULL,
	"instance_id" uuid,
	"ref_type" text NOT NULL,
	"ref_id" text,
	"status" text DEFAULT 'active' NOT NULL,
	"details" jsonb,
	"expires_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "inventory_holds_quantity_positive" CHECK ("inventory_holds"."quantity" > 0),
	CONSTRAINT "inventory_holds_kind_consistent" CHECK (("inventory_holds"."kind" = 'instance' and "inventory_holds"."instance_id" is not null) or ("inventory_holds"."kind" = 'stack' and "inventory_holds"."instance_id" is null))
);
--> statement-breakpoint
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
CREATE INDEX "inventory_instances_holder_idx" ON "inventory_instances" USING btree ("holder_type","holder_id");--> statement-breakpoint
CREATE INDEX "inventory_instances_good_type_idx" ON "inventory_instances" USING btree ("good_type");--> statement-breakpoint
CREATE UNIQUE INDEX "inventory_instances_id_good_unique" ON "inventory_instances" USING btree ("id","good_type");--> statement-breakpoint
CREATE INDEX "inventory_stacks_holder_idx" ON "inventory_stacks" USING btree ("holder_type","holder_id");--> statement-breakpoint
CREATE INDEX "inventory_holds_holder_idx" ON "inventory_holds" USING btree ("holder_type","holder_id","good_type");--> statement-breakpoint
CREATE INDEX "inventory_holds_ref_idx" ON "inventory_holds" USING btree ("ref_type","ref_id");--> statement-breakpoint
CREATE INDEX "inventory_holds_instance_idx" ON "inventory_holds" USING btree ("instance_id");--> statement-breakpoint
CREATE INDEX "inventory_poi_shares_grantee_idx" ON "inventory_poi_shares" USING btree ("grantee_type","grantee_id");--> statement-breakpoint
CREATE INDEX "inventory_pois_owner_idx" ON "inventory_pois" USING btree ("owner_type","owner_id");--> statement-breakpoint
CREATE INDEX "inventory_pois_system_idx" ON "inventory_pois" USING btree ("system");--> statement-breakpoint
CREATE INDEX "inventory_pois_scene_idx" ON "inventory_pois" USING btree ("scene");--> statement-breakpoint
CREATE INDEX "inventory_pois_visibility_idx" ON "inventory_pois" USING btree ("visibility");