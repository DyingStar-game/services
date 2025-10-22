-- PostgreSQL schema: systems table
-- id: primary key
-- name: text field

CREATE TABLE IF NOT EXISTS systems (
	id SERIAL PRIMARY KEY,
	name TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS stars (
    id SERIAL PRIMARY KEY,
    uuid UUID DEFAULT gen_random_uuid() UNIQUE,
    system_id INTEGER REFERENCES systems(id),
    name TEXT NOT NULL,
    mass_kg FLOAT NOT NULL
);

-- PostgreSQL schema: planets table
-- id: primary key
-- system_id: foreign key to systems table
-- name: text field
CREATE TABLE IF NOT EXISTS planets (
    id SERIAL PRIMARY KEY,
    uuid UUID DEFAULT gen_random_uuid() UNIQUE,
    system_id INTEGER REFERENCES systems(id),
    name TEXT NOT NULL,
    internal_name TEXT NOT NULL,
    mass_kg FLOAT NOT NULL,
    periapsis_AU FLOAT NOT NULL,
    apoapsis_AU FLOAT NOT NULL,
    inc_deg FLOAT NOT NULL,
    node_deg FLOAT NOT NULL,
    arg_peri_deg FLOAT NOT NULL,
    mean_anomaly_deg FLOAT NOT NULL,
    radius_km FLOAT NOT NULL,
    radius_gravity_influence_km FLOAT NOT NULL
);

CREATE TABLE IF NOT EXISTS planet_moons (
    id SERIAL PRIMARY KEY,
    planet_id INTEGER REFERENCES planets(id),
    uuid UUID DEFAULT gen_random_uuid() UNIQUE,
    name TEXT NOT NULL,
    internal_name TEXT NOT NULL,
    mass_kg FLOAT NOT NULL,
    periapsis_AU FLOAT NOT NULL,
    apoapsis_AU FLOAT NOT NULL,
    inc_deg FLOAT NOT NULL,
    node_deg FLOAT NOT NULL,
    arg_peri_deg FLOAT NOT NULL,
    mean_anomaly_deg FLOAT NOT NULL,
    radius_km FLOAT NOT NULL,
    radius_gravity_influence_km FLOAT NOT NULL
);
