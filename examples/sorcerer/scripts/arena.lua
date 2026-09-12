-- Arena logic: routes input to the player, and resolves spell hits.
--
-- Kept in one place so the per-entity scripts stay small, and so the ordering
-- of hit resolution is explicit rather than emergent.

function on_ready(self)
  self.kills = 0
  self.bolts_fired = 0
end

function on_tick(self)
  local player = scene.find("/Arena01/Player")
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
