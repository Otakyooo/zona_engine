-- Сеточный инвентарь: предметы занимают несколько клеток и могут быть
-- повёрнуты. Старые строки получают (0,0); при загрузке сервер разводит
-- пересечения через zn_sim::normalize_items.
ALTER TABLE inventory_items ADD COLUMN x INTEGER NOT NULL DEFAULT 0;
ALTER TABLE inventory_items ADD COLUMN y INTEGER NOT NULL DEFAULT 0;
ALTER TABLE inventory_items ADD COLUMN rotated INTEGER NOT NULL DEFAULT 0;

ALTER TABLE stash_items ADD COLUMN x INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stash_items ADD COLUMN y INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stash_items ADD COLUMN rotated INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stash_items ADD COLUMN slot INTEGER NOT NULL DEFAULT 0;
