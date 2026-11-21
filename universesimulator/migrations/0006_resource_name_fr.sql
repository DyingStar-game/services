-- A resource's name in French, for the missions' French texts (title_i18n): NULL, its name.
ALTER TABLE resources ADD COLUMN IF NOT EXISTS name_fr TEXT;
