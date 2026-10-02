ALTER TABLE "missions" ADD COLUMN "visibility" text DEFAULT 'public' NOT NULL;--> statement-breakpoint
ALTER TABLE "missions" ADD COLUMN "escrow_status" text DEFAULT 'none' NOT NULL;--> statement-breakpoint
ALTER TABLE "missions" ADD COLUMN "escrow_amount" integer;--> statement-breakpoint
ALTER TABLE "missions" ADD COLUMN "escrow_currency" text;--> statement-breakpoint
ALTER TABLE "missions" ADD COLUMN "escrow_payer_id" uuid;--> statement-breakpoint
ALTER TABLE "missions" ADD COLUMN "escrow_external_id" text;--> statement-breakpoint
ALTER TABLE "mission_assignments" ADD COLUMN "reward_amount" integer;--> statement-breakpoint
CREATE UNIQUE INDEX "missions_escrow_external_id_unique" ON "missions" USING btree ("escrow_external_id");--> statement-breakpoint
CREATE INDEX "missions_visibility_status_idx" ON "missions" USING btree ("visibility","status");