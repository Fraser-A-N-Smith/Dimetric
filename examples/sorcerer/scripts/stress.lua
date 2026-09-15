-- The density §12 asks the engine to survive, as a scene rather than a claim.
--
-- Forty enemies on a ring, and a projectile budget kept topped up every tick.
-- Nothing here is a game: it exists so the spatial hash and the Lua boundary
-- are measured at the density the slice is supposed to reach, rather than at
-- the density a recorded play session happened to produce.

local ENEMIES = 40
local PROJECTILES = 400
local CENTRE = vec2(fx.new(160), fx.new(96))

function on_ready(self)
  self.spawned_enemies = false
  self.fired = 0
end

function on_tick(self)
  if not self.spawned_enemies then
    self.spawned_enemies = true
    for i = 0, ENEMIES - 1 do
      local degrees = (i * 9) % 360
      local ring = fx.new(60 + (i % 4) * 12)
      scene.spawn("prefabs/skeleton", CENTRE + fx.from_angle(tostring(degrees)) * ring)
    end
    return
  end

  -- Top the projectile count back up to the budget. They expire on their own,
  -- so this settles at roughly the budget rather than growing without bound.
  local live = #scene.tagged("projectile")
  self.live = live
  local wanted = PROJECTILES - live
  if wanted > 60 then wanted = 60 end
  for i = 1, wanted do
    local degrees = ((self.fired or 0) * 13 + i * 7) % 360
    local id = scene.spawn("prefabs/bolt", CENTRE)
    local pending = self.pending or {}
    pending[#pending + 1] = { id = id, angle = degrees }
    self.pending = pending
    self.fired = (self.fired or 0) + 1
  end

  -- Arm whatever arrived last tick.
  local pending = self.pending
  if pending then
    local waiting = {}
    for i = 1, #pending do
      local node = scene.by_id(pending[i].id)
      if node then
        node.life = 240
        node.damage = 1
        node.speed = 40
        node.homing = 6
        node:set_velocity(fx.from_angle(tostring(pending[i].angle)) * fx.new(40))
      else
        waiting[#waiting + 1] = pending[i]
      end
    end
    self.pending = waiting
  end
end

function on_post_tick(self)
  self.enemy_count = #scene.tagged("enemy")
  self.live_projectiles = #scene.tagged("projectile")
end
