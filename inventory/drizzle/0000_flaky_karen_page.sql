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
CREATE INDEX "inventory_instances_holder_idx" ON "inventory_instances" USING btree ("holder_type","holder_id");--> statement-breakpoint
CREATE INDEX "inventory_instances_good_type_idx" ON "inventory_instances" USING btree ("good_type");--> statement-breakpoint
CREATE UNIQUE INDEX "inventory_instances_id_good_unique" ON "inventory_instances" USING btree ("id","good_type");--> statement-breakpoint
CREATE INDEX "inventory_stacks_holder_idx" ON "inventory_stacks" USING btree ("holder_type","holder_id");--> statement-breakpoint
CREATE INDEX "inventory_holds_holder_idx" ON "inventory_holds" USING btree ("holder_type","holder_id","good_type");--> statement-breakpoint
CREATE INDEX "inventory_holds_ref_idx" ON "inventory_holds" USING btree ("ref_type","ref_id");--> statement-breakpoint
CREATE INDEX "inventory_holds_instance_idx" ON "inventory_holds" USING btree ("instance_id");