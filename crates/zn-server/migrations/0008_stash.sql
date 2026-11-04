-- Схрон: вынесенный из рейда лут.
--
-- Раньше эвакуация писала инвентарь в `inventory_items` и очищала его, но
-- периодическое сохранение через минуту записывало туда уже пустой инвентарь
-- и стирало награду. Схрон — отдельная таблица, поэтому очистка рейда его
-- не трогает.
CREATE TABLE IF NOT EXISTS stash_items (
    user_id INTEGER NOT NULL,
    item_id INTEGER NOT NULL,
    quantity INTEGER NOT NULL,
    PRIMARY KEY (user_id, item_id)
);
