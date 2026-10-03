ALTER TABLE "missions" ADD COLUMN "escrow_item_status" text DEFAULT 'none' NOT NULL;--> statement-breakpoint
ALTER TABLE "missions" ADD COLUMN "escrow_item_hold_id" uuid;--> statement-breakpoint
ALTER TABLE "mission_assignments" ADD COLUMN "reward_item_quantity" integer;--> statement-breakpoint
ALTER TABLE "mission_assignments" ADD COLUMN "item_settled" boolean DEFAULT false NOT NULL;--> statement-breakpoint
ALTER TABLE "mission_assignments" ADD COLUMN "item_settled_at" timestamp with time zone;--> statement-breakpoint
ALTER TABLE "mission_assignments" ADD COLUMN "item_details" jsonb;