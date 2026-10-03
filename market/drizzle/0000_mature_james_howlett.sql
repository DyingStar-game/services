CREATE TABLE "market_catalog" (
	"good_type" text PRIMARY KEY NOT NULL,
	"kind" text NOT NULL,
	"unit" text,
	"display_name" text,
	"enabled" boolean DEFAULT true NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "market_demands" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"good_type" text NOT NULL,
	"kind" text NOT NULL,
	"instance_id" uuid,
	"quantity" bigint NOT NULL,
	"max_price" bigint NOT NULL,
	"currency" text DEFAULT 'credits' NOT NULL,
	"holder_type" text NOT NULL,
	"holder_id" uuid NOT NULL,
	"is_corporation" boolean DEFAULT false NOT NULL,
	"message" text,
	"status" text DEFAULT 'open' NOT NULL,
	"created_by" text,
	"expires_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "market_demands_quantity_positive" CHECK ("market_demands"."quantity" > 0),
	CONSTRAINT "market_demands_max_price_non_negative" CHECK ("market_demands"."max_price" >= 0)
);
--> statement-breakpoint
CREATE TABLE "market_orders" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"side" text NOT NULL,
	"good_type" text NOT NULL,
	"kind" text NOT NULL,
	"instance_id" uuid,
	"quantity" bigint NOT NULL,
	"remaining" bigint NOT NULL,
	"price" bigint NOT NULL,
	"currency" text DEFAULT 'credits' NOT NULL,
	"holder_type" text NOT NULL,
	"holder_id" uuid NOT NULL,
	"is_corporation" boolean DEFAULT false NOT NULL,
	"status" text DEFAULT 'open' NOT NULL,
	"created_by" text,
	"expires_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "market_orders_quantity_positive" CHECK ("market_orders"."quantity" > 0),
	CONSTRAINT "market_orders_price_non_negative" CHECK ("market_orders"."price" >= 0),
	CONSTRAINT "market_orders_buy_is_stack" CHECK ("market_orders"."side" = 'sell' or "market_orders"."kind" = 'stack')
);
--> statement-breakpoint
CREATE TABLE "market_trades" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"good_type" text NOT NULL,
	"kind" text NOT NULL,
	"instance_id" uuid,
	"quantity" bigint NOT NULL,
	"unit_price" bigint NOT NULL,
	"total_price" bigint NOT NULL,
	"currency" text DEFAULT 'credits' NOT NULL,
	"buyer_type" text NOT NULL,
	"buyer_id" uuid NOT NULL,
	"seller_type" text NOT NULL,
	"seller_id" uuid NOT NULL,
	"buy_order_id" uuid,
	"sell_order_id" uuid,
	"demand_id" uuid,
	"status" text DEFAULT 'pending' NOT NULL,
	"goods_moved" boolean DEFAULT false NOT NULL,
	"money_moved" boolean DEFAULT false NOT NULL,
	"failure_reason" text,
	"details" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"settled_at" timestamp with time zone,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "market_trades_quantity_positive" CHECK ("market_trades"."quantity" > 0)
);
--> statement-breakpoint
CREATE INDEX "market_demands_open_idx" ON "market_demands" USING btree ("good_type","status");--> statement-breakpoint
CREATE INDEX "market_demands_holder_idx" ON "market_demands" USING btree ("holder_type","holder_id");--> statement-breakpoint
CREATE INDEX "market_orders_book_idx" ON "market_orders" USING btree ("good_type","side","status","price");--> statement-breakpoint
CREATE INDEX "market_orders_holder_idx" ON "market_orders" USING btree ("holder_type","holder_id");--> statement-breakpoint
CREATE INDEX "market_orders_instance_idx" ON "market_orders" USING btree ("instance_id");--> statement-breakpoint
CREATE INDEX "market_trades_status_idx" ON "market_trades" USING btree ("status");--> statement-breakpoint
CREATE INDEX "market_trades_buyer_idx" ON "market_trades" USING btree ("buyer_type","buyer_id");--> statement-breakpoint
CREATE INDEX "market_trades_seller_idx" ON "market_trades" USING btree ("seller_type","seller_id");