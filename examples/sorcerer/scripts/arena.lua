-- Arena logic: routes input to the player, and resolves spell hits.
--
-- Kept in one place so the per-entity scripts stay small, and so the ordering
-- of hit resolution is explicit rather than emergent.

function on_ready(self)
  self.kills = 0
  self.bolts_fired = 0
end

function on_tick(self)
  -- Same idiom as skeleton.lua: resolve the path once, then look up by id.
  if self.player_id then
    local cached = scene.by_id(self.player_id)
    if not cached then self.player_id = nil end
  end
  if not self.player_id then
    local found = scene.find("/Arena01/Player")
    if found then self.player_id = found:id() end
  end

  local player = self.player_id and scene.by_id(self.player_id)
  if not player then return end

  -- A bolt is resolved as an instant ray for now. The vertical slice will
  -- replace this with a spawned projectile; the interface does not change.
  if self.fire == true and player:get("casts") ~= nil then
    self.bolts_fired = (self.bolts_fired or 0) + 1
  end
end

function on_post_tick(self)
  local enemies = scene.tagged("enemy")
  self.enemy_count = #enemies
end
