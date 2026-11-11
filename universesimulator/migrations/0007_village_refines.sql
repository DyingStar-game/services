-- Whether a village has a refinery (the factory cities): a mission that has ore refined on the way
-- names the refining village nearest to its destination.
ALTER TABLE villages ADD COLUMN IF NOT EXISTS refines BOOLEAN NOT NULL DEFAULT FALSE;

-- What is mined to make a resource, as the missions' texts say it ("gold" / "de l'or"): crushed gold ore is
-- not mined, gold is, then crushed. NULL: the texts do not name it.
ALTER TABLE resources ADD COLUMN IF NOT EXISTS mined TEXT;
ALTER TABLE resources ADD COLUMN IF NOT EXISTS mined_fr TEXT;
