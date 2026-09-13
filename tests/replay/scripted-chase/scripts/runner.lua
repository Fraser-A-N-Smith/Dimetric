-- Walks toward the target and announces its arrival once.

local SPEED = fx.new(60)

-- The path is resolved once and the id kept, which is the idiom every script
-- that chases something should use. See `scripts/skeleton.lua` in the sorcerer
-- example for why, and `cargo bench -p dimetric-sim` for the numbers.
local function find_target(self)
  if self.target_id then
    local cached = scene.by_id(self.target_id)
    if cached then return cached end
  end
  local target = scene.find("/Arena/Target")
  if target then
    self.target_id = target:id()
  end
  return target
end

function on_ready(self)
  self.arrived = false
  -- A seeded roll, so the fixture would catch a change to the RNG or to the
  -- order streams are drawn in.
  self.jitter = rng.range("spawn", 0, 16)
  find_target(self)
end

function on_tick(self)
  if self.arrived == true then return end

  local target = find_target(self)
  if not target then return end

  local to = target.pos - self.pos
  if to:length() < fx.new(8) then
    self.arrived = true
    self:set_velocity(vec2(fx.new(0), fx.new(0)))
    self:emit("arrived", { jitter = self.jitter })
  else
    self:set_velocity(to:normalized() * SPEED)
  end
end
