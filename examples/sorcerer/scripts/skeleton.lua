-- A skeleton that walks at the player and dies when its health runs out.

local SPEED = fx.new(35)

function on_ready(self)
  self.health = 40
  self.hits = 0
end

function on_tick(self)
  local player = scene.find("/Arena01/Player")
  if not player then return end

  local to = player.pos - self.pos
  -- Squared comparison: no square root, and exact in the accumulator.
  if to:length() > fx.new(12) then
    self:set_velocity(to:normalized() * SPEED)
  else
    self:set_velocity(vec2(fx.new(0), fx.new(0)))
  end
end

-- Called by the arena when a bolt overlaps this skeleton's hurtbox.
function take_damage(self, amount)
  self.health = self.health - amount
  self.hits = (self.hits or 0) + 1
  if self.health <= 0 then
    self:emit("died", { hits = self.hits })
    self:destroy()
  end
end
