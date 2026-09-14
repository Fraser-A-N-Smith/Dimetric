-- One pooled projectile.
--
-- Inactive when `life` is zero or absent: parked far off the arena so it sits
-- in a spatial-hash cell nothing else occupies, and invisible so it costs a
-- sprite but never appears. A projectile is never created or destroyed at
-- runtime, only taken and returned — see the note in `arena.lua`.

local PARKED = vec2(fx.new(-9000), fx.new(-9000))

local function park(self)
  self.life = 0
  self.visible = false
  self.pos = PARKED
  self:set_velocity(vec2(fx.new(0), fx.new(0)))
end

function on_ready(self)
  park(self)
end

function on_tick(self)
  local life = self.life or 0
  if life <= 0 then return end

  life = life - 1
  self.life = life
  if life <= 0 then
    park(self)
    return
  end

  -- Homing turns the velocity toward the nearest enemy by a fixed number of
  -- degrees a tick. A fixed step rather than a proportion, so the turn rate is
  -- a whole number and the arc is the same on every machine.
  local homing = self.homing or 0
  if homing > 0 then
    local target = nearest_enemy(self)
    if target then
      local to = target.pos - self.pos
      if to:length() > fx.new(1) then
        local want = to:angle_degrees()
        local have = self:velocity():angle_degrees()
        local turned = turn_toward(have, want, homing)
        self:set_velocity(fx.from_angle(turned) * fx.new(self.speed or 60))
      end
    end
  end
end

-- The closest tagged enemy, or nothing.
--
-- Linear in enemy count, which is fine at the dozens the slice runs and is the
-- kind of thing a broadphase query would replace. Logged as a gap rather than
-- built mid-slice.
function nearest_enemy(self)
  local enemies = scene.tagged("enemy")
  local best, best_distance = nil, nil
  for i = 1, #enemies do
    local d = (enemies[i].pos - self.pos):length_squared()
    if not best_distance or d < best_distance then
      best, best_distance = enemies[i], d
    end
  end
  return best
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

  -- Damage is written straight into the other node's script state. The engine
  -- has no way for one script to call a function on another, so the convention
  -- is that an enemy reads `pending_damage` on its own tick.
  other.pending_damage = (other.pending_damage or 0) + (self.damage or 0)

  -- Orbiting spells persist through a hit; everything else is spent.
  if (self.orbit_radius or 0) <= 0 then
    park(self)
  end
end
