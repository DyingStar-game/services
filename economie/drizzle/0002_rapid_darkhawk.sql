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
ALTER TABLE "corporation_member_salaries" ADD CONSTRAINT "corporation_member_salaries_corporation_id_player_id_corporation_members_corporation_id_player_id_fk" FOREIGN KEY ("corporation_id","player_id") REFERENCES "public"."corporation_members"("corporation_id","player_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "corporation_member_salaries_player_idx" ON "corporation_member_salaries" USING btree ("player_id");