-- A skeleton that walks at the player and dies when its health runs out.

local SPEED = fx.new(35)

-- Resolve the player's path once and keep its id.
--
-- `scene.find` walks the sibling list matching each name, so calling it every
-- tick from every enemy is quadratic in enemy count: about 3.9us a call against
-- 140ns for a handle validation, and worse the further down the list the target
-- sits. `scene.by_id` skips the walk.
--
-- The id, not the handle: a handle stored in a script variable is converted to
-- its id string, because script state has to survive a snapshot and a userdata
-- pointer cannot. Re-resolving on a miss keeps this correct if the player is
-- ever destroyed and respawned.
--
-- See `cargo bench -p dimetric-sim` for the measurements.
local function find_player(self)
  if self.player_id then
    local cached = scene.by_id(self.player_id)
    if cached then return cached end
  end
  local player = scene.find("/Arena01/Player")
  if player then
    self.player_id = player:id()
  end
  return player
end

function on_ready(self)
  self.health = 40
  self.hits = 0
  find_player(self)
end

function on_tick(self)
  local player = find_player(self)
  if not player then return end

  local to = player.pos - self.pos
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
