ALTER TABLE "inventory_stacks" DROP CONSTRAINT "inventory_stacks_holder_type_valid";--> statement-breakpoint
ALTER TABLE "inventory_pois" DROP CONSTRAINT "inventory_pois_owner_type_valid";--> statement-breakpoint
ALTER TABLE "inventory_stacks" ADD COLUMN "carried" bigint DEFAULT 0 NOT NULL;--> statement-breakpoint
ALTER TABLE "inventory_pois" ADD COLUMN "created_by" uuid;--> statement-breakpoint
ALTER TABLE "inventory_stacks" ADD CONSTRAINT "inventory_stacks_carried_positive" CHECK ("inventory_stacks"."carried" >= 0);--> statement-breakpoint
ALTER TABLE "inventory_stacks" ADD CONSTRAINT "inventory_stacks_carried_bounded" CHECK ("inventory_stacks"."carried" <= "inventory_stacks"."quantity");--> statement-breakpoint
ALTER TABLE "inventory_stacks" ADD CONSTRAINT "inventory_stacks_holder_type_valid" CHECK ("inventory_stacks"."holder_type" in ('player','npc','corporation','political','system','warehouse'));--> statement-breakpoint
ALTER TABLE "inventory_pois" ADD CONSTRAINT "inventory_pois_owner_type_valid" CHECK ("inventory_pois"."owner_type" in ('player','npc','corporation','political','system','mission'));--> statement-breakpoint
INSERT INTO "inventory_good_types" ("good_type", "kind") VALUES ('ore.gold', 'stack') ON CONFLICT ("good_type") DO NOTHING;--> statement-breakpoint
INSERT INTO "inventory_good_types" ("good_type", "kind") VALUES ('ore.iron', 'stack') ON CONFLICT ("good_type") DO NOTHING;--> statement-breakpoint
INSERT INTO "inventory_good_types" ("good_type", "kind") VALUES ('ore.cryptonite', 'stack') ON CONFLICT ("good_type") DO NOTHING;