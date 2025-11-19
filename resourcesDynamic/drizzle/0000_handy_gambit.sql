CREATE TABLE "systems" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "systems_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"uuid" uuid DEFAULT gen_random_uuid(),
	"name" text NOT NULL,
	"internal_name" text NOT NULL,
	CONSTRAINT "systems_uuid_unique" UNIQUE("uuid"),
	CONSTRAINT "systems_name_unique" UNIQUE("name"),
	CONSTRAINT "systems_internal_name_unique" UNIQUE("internal_name")
);
--> statement-breakpoint
CREATE TABLE "stars" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "stars_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"uuid" uuid DEFAULT gen_random_uuid(),
	"system_id" integer,
	"name" text NOT NULL,
	"internal_name" text NOT NULL,
	"mass_kg" double precision NOT NULL,
	CONSTRAINT "stars_uuid_unique" UNIQUE("uuid"),
	CONSTRAINT "stars_name_unique" UNIQUE("name"),
	CONSTRAINT "stars_internal_name_unique" UNIQUE("internal_name")
);
--> statement-breakpoint
CREATE TABLE "planets" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "planets_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"uuid" uuid DEFAULT gen_random_uuid(),
	"system_id" integer,
	"name" text NOT NULL,
	"internal_name" text NOT NULL,
	"mass_kg" double precision NOT NULL,
	"periapsis_au" double precision NOT NULL,
	"apoapsis_au" double precision NOT NULL,
	"inc_rad" double precision NOT NULL,
	"node_rad" double precision NOT NULL,
	"arg_peri_rad" double precision NOT NULL,
	"mean_anomaly_rad" double precision NOT NULL,
	"radius_m" double precision NOT NULL,
	"radius_gravity_influence_m" double precision NOT NULL,
	"tidal_locked" boolean NOT NULL,
	"tilt_rad" double precision NOT NULL,
	"rotation_h" double precision NOT NULL,
	CONSTRAINT "planets_uuid_unique" UNIQUE("uuid"),
	CONSTRAINT "planets_name_unique" UNIQUE("name"),
	CONSTRAINT "planets_internal_name_unique" UNIQUE("internal_name")
);
--> statement-breakpoint
CREATE TABLE "planet_moons" (
	"id" integer PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "planet_moons_id_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 2147483647 START WITH 1 CACHE 1),
	"planet_id" integer,
	"uuid" uuid DEFAULT gen_random_uuid(),
	"name" text NOT NULL,
	"internal_name" text NOT NULL,
	"mass_kg" double precision NOT NULL,
	"periapsis_au" double precision NOT NULL,
	"apoapsis_au" double precision NOT NULL,
	"inc_rad" double precision NOT NULL,
	"node_rad" double precision NOT NULL,
	"arg_peri_rad" double precision NOT NULL,
	"mean_anomaly_rad" double precision NOT NULL,
	"radius_m" double precision NOT NULL,
	"radius_gravity_influence_m" double precision NOT NULL,
	"tidal_locked" boolean NOT NULL,
	"tilt_rad" double precision NOT NULL,
	"rotation_h" double precision NOT NULL,
	CONSTRAINT "planet_moons_uuid_unique" UNIQUE("uuid"),
	CONSTRAINT "planet_moons_name_unique" UNIQUE("name"),
	CONSTRAINT "planet_moons_internal_name_unique" UNIQUE("internal_name")
);
--> statement-breakpoint
ALTER TABLE "stars" ADD CONSTRAINT "stars_system_id_systems_id_fk" FOREIGN KEY ("system_id") REFERENCES "public"."systems"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "planets" ADD CONSTRAINT "planets_system_id_systems_id_fk" FOREIGN KEY ("system_id") REFERENCES "public"."systems"("id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "planet_moons" ADD CONSTRAINT "planet_moons_planet_id_planets_id_fk" FOREIGN KEY ("planet_id") REFERENCES "public"."planets"("id") ON DELETE no action ON UPDATE no action;