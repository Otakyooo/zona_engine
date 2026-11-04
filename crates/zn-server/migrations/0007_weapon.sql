-- Выбранное оружие и остаток патронов — часть прогресса.
-- Значения по умолчанию совпадают с `zn_sim::items`: пистолет и 90 патронов.
ALTER TABLE profiles ADD COLUMN weapon_id INTEGER NOT NULL DEFAULT 10;
ALTER TABLE profiles ADD COLUMN ammo INTEGER NOT NULL DEFAULT 90;
