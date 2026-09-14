-- One spell projectile.
--
-- Spawned by the arena, armed on its first tick, and destroyed when its life
-- runs out or it hits something. Nothing is pooled: the engine can create and
-- destroy nodes, so a projectile is a node for exactly as long as it exists.

function on_tick(self)
  local life = self.life or 0
  -- Unarmed: the arena writes the numbers on the tick after the spawn lands.
  if life <= 0 then
    if self.armed then self:destroy() end
    return
  end
  self.armed = true

  life = life - 1
  self.life = life
  if life <= 0 then
    self:destroy()
    return
  end

  -- Homing turns the velocity toward the nearest enemy by a whole number of
  -- degrees a tick, so the arc is the same on every machine.
  local homing = self.homing or 0
  if homing > 0 then
    local target = scene.nearest(self.pos, fx.new(160), "enemy")
    if target then
      local to = target.pos - self.pos
      if to:length() > fx.new(1) then
        local turned = turn_toward(self:velocity():angle_degrees(), to:angle_degrees(), homing)
        self:set_velocity(fx.from_angle(turned) * fx.new(self.speed or 60))
      end
    end
  end
end

-- Rotate `have` toward `want` by at most `step` degrees, the short way round.
function turn_toward(have, want, step)
  local h = tonumber(have) or 0
  local w = tonumber(want) or 0
  local delta = (w - h) % 360
  if delta > 180 then delta = delta - 360 end
  if delta > step then delta = step end
  if delta < -step then delta = -step end
  return tostring((h + delta) % 360)
end

function on_collide(self, other, normal, trigger)
  if (self.life or 0) <= 0 then return end
  if not other:has_tag("enemy") then return end

  -- Damage is written into the other node's own state and read on its next
  -- tick. The engine has no way for one script to call a function on another,
  -- and the indirection is arguably better: the ordering stays explicit and it
  -- survives the target being destroyed mid-frame.
  other.pending_damage = (other.pending_damage or 0) + (self.damage or 0)

  -- Orbiting spells persist through a hit; everything else is spent.
  if (self.orbit_radius or 0) <= 0 then
    self:destroy()
  end
end
