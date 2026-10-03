CREATE TABLE "political_members" (
	"entity_id" uuid NOT NULL,
	"player_id" uuid NOT NULL,
	"holder_type" text DEFAULT 'player' NOT NULL,
	"role" text DEFAULT 'member' NOT NULL,
	"joined_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "political_members_entity_id_player_id_pk" PRIMARY KEY("entity_id","player_id")
);
--> statement-breakpoint
CREATE TABLE "political_settings" (
	"entity_id" uuid PRIMARY KEY NOT NULL,
	"corporate_tax_bps" integer DEFAULT 0 NOT NULL,
	"income_tax_bps" integer DEFAULT 0 NOT NULL,
	"allow_minting" boolean DEFAULT false NOT NULL,
	"mint_ceiling" bigint DEFAULT 0 NOT NULL,
	"last_assessed_at" timestamp with time zone,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "political_settings_corporate_tax_bps_range" CHECK ("political_settings"."corporate_tax_bps" >= 0 AND "political_settings"."corporate_tax_bps" <= 10000),
	CONSTRAINT "political_settings_income_tax_bps_range" CHECK ("political_settings"."income_tax_bps" >= 0 AND "political_settings"."income_tax_bps" <= 10000),
	CONSTRAINT "political_settings_mint_ceiling_positive" CHECK ("political_settings"."mint_ceiling" >= 0)
);
--> statement-breakpoint
CREATE TABLE "political_tax_debts" (
	"id" bigint PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "political_tax_debts_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 9223372036854775807 START WITH 1 CACHE 1),
	"assessment_id" uuid NOT NULL,
	"entity_id" uuid NOT NULL,
	"debtor_type" text NOT NULL,
	"debtor_id" uuid NOT NULL,
	"currency" text DEFAULT 'credits' NOT NULL,
	"amount" bigint NOT NULL,
	"tax_type" text NOT NULL,
	"status" text DEFAULT 'due' NOT NULL,
	"details" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"paid_at" timestamp with time zone,
	CONSTRAINT "political_tax_debts_assessment_unique" UNIQUE("assessment_id","debtor_type","debtor_id"),
	CONSTRAINT "political_tax_debts_amount_positive" CHECK ("political_tax_debts"."amount" > 0)
);
--> statement-breakpoint
ALTER TABLE "corporation_settings" ADD COLUMN "political_entity_id" uuid;--> statement-breakpoint
CREATE INDEX "political_members_player_idx" ON "political_members" USING btree ("player_id");--> statement-breakpoint
CREATE INDEX "political_tax_debts_debtor_idx" ON "political_tax_debts" USING btree ("debtor_type","debtor_id","status");--> statement-breakpoint
CREATE INDEX "political_tax_debts_entity_idx" ON "political_tax_debts" USING btree ("entity_id","status");