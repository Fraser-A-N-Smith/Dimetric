-- Walks toward the target and announces its arrival once.

local SPEED = fx.new(60)

function on_ready(self)
  self.arrived = false
  -- A seeded roll, so the fixture would catch a change to the RNG or to the
  -- order streams are drawn in.
  self.jitter = rng.range("spawn", 0, 16)
end

function on_tick(self)
  local target = scene.find("/Arena/Target")
  if not target or self.arrived == true then return end

  local to = target.pos - self.pos
  if to:length() < fx.new(8) then
    self.arrived = true
    self:set_velocity(vec2(fx.new(0), fx.new(0)))
    self:emit("arrived", { jitter = self.jitter })
  else
    self:set_velocity(to:normalized() * SPEED)
  end
end
