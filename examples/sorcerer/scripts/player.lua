-- The sorcerer. Moves on the stick, casts on the fire button.
--
-- Every number that reaches simulation state goes through `fx` or `vec2`.
-- Raw Lua arithmetic here would be a replay divergence waiting for a
-- different machine to find it.

local SPEED = fx.new(90)
local CAST_COOLDOWN = 12

function on_ready(self)
  self.cooldown = 0
  self.casts = 0
end

function on_tick(self)
  self:set_velocity(input_direction(self) * SPEED)

  if self.cooldown > 0 then
    self.cooldown = self.cooldown - 1
  end
end

-- The arena reads this to decide where to spawn a bolt.
function aim_direction(self)
  return fx.from_angle(self.aim or "0.0")
end

function input_direction(self)
  -- Held by the arena each tick, so the player script never touches raw input.
  return self.move or vec2(fx.new(0), fx.new(0))
end

function can_cast(self)
  return self.cooldown <= 0
end

function note_cast(self)
  self.cooldown = CAST_COOLDOWN
  self.casts = (self.casts or 0) + 1
end
