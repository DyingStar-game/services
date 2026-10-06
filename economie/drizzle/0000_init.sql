CREATE TABLE "accounts" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"holder_type" text NOT NULL,
	"holder_id" uuid NOT NULL,
	"currency" text DEFAULT 'credits' NOT NULL,
	"balance" bigint DEFAULT 0 NOT NULL,
	"status" text DEFAULT 'active' NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "accounts_holder_currency_unique" UNIQUE("holder_type","holder_id","currency"),
	CONSTRAINT "accounts_balance_positive" CHECK ("accounts"."balance" >= 0)
);
--> statement-breakpoint
CREATE TABLE "transactions" (
	"id" bigint PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "transactions_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 9223372036854775807 START WITH 1 CACHE 1),
	"from_account_id" uuid,
	"to_account_id" uuid,
	"type" text NOT NULL,
	"currency" text DEFAULT 'credits' NOT NULL,
	"amount" bigint NOT NULL,
	"tax_amount" bigint DEFAULT 0 NOT NULL,
	"fee_amount" bigint DEFAULT 0 NOT NULL,
	"external_id" text,
	"reference" text,
	"caller" text,
	"details" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "transactions_external_id_unique" UNIQUE("external_id"),
	CONSTRAINT "transactions_amount_positive" CHECK ("transactions"."amount" > 0)
);
--> statement-breakpoint
CREATE TABLE "corporation_members" (
	"corporation_id" uuid NOT NULL,
	"player_id" uuid NOT NULL,
	"holder_type" text DEFAULT 'player' NOT NULL,
	"role" text DEFAULT 'member' NOT NULL,
	"joined_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "corporation_members_corporation_id_player_id_pk" PRIMARY KEY("corporation_id","player_id")
);
--> statement-breakpoint
CREATE TABLE "corporation_settings" (
	"corporation_id" uuid PRIMARY KEY NOT NULL,
	"tax_rate_bps" integer DEFAULT 0 NOT NULL,
	"allow_donations" boolean DEFAULT true NOT NULL,
	"political_entity_id" uuid,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "corporation_member_salaries" (
	"corporation_id" uuid NOT NULL,
	"player_id" uuid NOT NULL,
	"currency" text DEFAULT 'credits' NOT NULL,
	"amount" bigint DEFAULT 0 NOT NULL,
	"enabled" boolean DEFAULT true NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "corporation_member_salaries_corporation_id_player_id_currency_pk" PRIMARY KEY("corporation_id","player_id","currency"),
	CONSTRAINT "corporation_member_salaries_amount_positive" CHECK ("corporation_member_salaries"."amount" >= 0)
);
--> statement-breakpoint
CREATE TABLE "corporation_salary_roles" (
	"corporation_id" uuid NOT NULL,
	"role" text NOT NULL,
	"currency" text DEFAULT 'credits' NOT NULL,
	"amount" bigint DEFAULT 0 NOT NULL,
	"enabled" boolean DEFAULT true NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "corporation_salary_roles_corporation_id_role_currency_pk" PRIMARY KEY("corporation_id","role","currency"),
	CONSTRAINT "corporation_salary_roles_amount_positive" CHECK ("corporation_salary_roles"."amount" >= 0)
);
--> statement-breakpoint
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
ALTER TABLE "transactions" ADD CONSTRAINT "transactions_from_account_id_accounts_id_fk" FOREIGN KEY ("from_account_id") REFERENCES "public"."accounts"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "transactions" ADD CONSTRAINT "transactions_to_account_id_accounts_id_fk" FOREIGN KEY ("to_account_id") REFERENCES "public"."accounts"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "corporation_member_salaries" ADD CONSTRAINT "corporation_member_salaries_corporation_id_player_id_corporation_members_corporation_id_player_id_fk" FOREIGN KEY ("corporation_id","player_id") REFERENCES "public"."corporation_members"("corporation_id","player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "accounts_holder_idx" ON "accounts" USING btree ("holder_type","holder_id");--> statement-breakpoint
CREATE INDEX "transactions_from_created_idx" ON "transactions" USING btree ("from_account_id","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "transactions_to_created_idx" ON "transactions" USING btree ("to_account_id","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "transactions_created_idx" ON "transactions" USING btree ("created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "corporation_members_player_idx" ON "corporation_members" USING btree ("player_id");--> statement-breakpoint
CREATE INDEX "corporation_member_salaries_player_idx" ON "corporation_member_salaries" USING btree ("player_id");--> statement-breakpoint
CREATE INDEX "political_members_player_idx" ON "political_members" USING btree ("player_id");--> statement-breakpoint
CREATE INDEX "political_tax_debts_debtor_idx" ON "political_tax_debts" USING btree ("debtor_type","debtor_id","status");--> statement-breakpoint
CREATE INDEX "political_tax_debts_entity_idx" ON "political_tax_debts" USING btree ("entity_id","status");