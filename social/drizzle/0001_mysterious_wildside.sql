ALTER TABLE "corporation_members" DROP CONSTRAINT "corporation_members_player_unique";--> statement-breakpoint
ALTER TABLE "corporations" ADD COLUMN "parent_id" uuid;--> statement-breakpoint
ALTER TABLE "corporations" ADD CONSTRAINT "corporations_parent_id_corporations_id_fk" FOREIGN KEY ("parent_id") REFERENCES "public"."corporations"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "corporation_members_player_idx" ON "corporation_members" USING btree ("player_id");--> statement-breakpoint
CREATE INDEX "corporations_parent_idx" ON "corporations" USING btree ("parent_id");