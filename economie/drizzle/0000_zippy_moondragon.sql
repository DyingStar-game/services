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
	"details" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "transactions_external_id_unique" UNIQUE("external_id"),
	CONSTRAINT "transactions_amount_positive" CHECK ("transactions"."amount" > 0)
);
--> statement-breakpoint
CREATE TABLE "corporation_members" (
	"corporation_id" uuid NOT NULL,
	"player_id" uuid NOT NULL,
	"role" text DEFAULT 'member' NOT NULL,
	"joined_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "corporation_members_corporation_id_player_id_pk" PRIMARY KEY("corporation_id","player_id")
);
--> statement-breakpoint
CREATE TABLE "corporation_settings" (
	"corporation_id" uuid PRIMARY KEY NOT NULL,
	"tax_rate_bps" integer DEFAULT 0 NOT NULL,
	"allow_donations" boolean DEFAULT true NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "transactions" ADD CONSTRAINT "transactions_from_account_id_accounts_id_fk" FOREIGN KEY ("from_account_id") REFERENCES "public"."accounts"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "transactions" ADD CONSTRAINT "transactions_to_account_id_accounts_id_fk" FOREIGN KEY ("to_account_id") REFERENCES "public"."accounts"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "accounts_holder_idx" ON "accounts" USING btree ("holder_type","holder_id");--> statement-breakpoint
CREATE INDEX "transactions_from_created_idx" ON "transactions" USING btree ("from_account_id","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "transactions_to_created_idx" ON "transactions" USING btree ("to_account_id","created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "transactions_created_idx" ON "transactions" USING btree ("created_at" DESC NULLS LAST);--> statement-breakpoint
CREATE INDEX "corporation_members_player_idx" ON "corporation_members" USING btree ("player_id");