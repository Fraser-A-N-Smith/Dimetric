-- An enemy that walks at the player and dies when its health runs out.
--
-- One script for every variant. What differs between a skeleton, a wraith and a
-- brute is numbers, and those are properties of the project's `Enemy` kind —
-- authored in the prefab, overridable per instance, and never in here.

local function find_player(self)
  if self.player_id then
    local cached = scene.by_id(self.player_id)
    if cached then return cached end
  end
  local player = scene.find("/Arena01/Player")
  if player then self.player_id = player:id() end
  return player
end

function on_ready(self)
  -- The numbers live on the node. A skeleton, a wraith and a brute are the
  -- same prefab shape with different authored properties, and an instance in a
  -- scene can override any of them without touching this file.
  self.health = self:get("max_health")
  self.speed = self:get("speed")
  self.touch_damage = self:get("touch_damage")
  self.hits = 0
  self.pending_damage = 0
end

function on_tick(self)
  -- Damage first, so a kill this tick does not also get to move.
  local pending = self.pending_damage or 0
  if pending > 0 then
    self.pending_damage = 0
    self.health = (self.health or 0) - pending
    self.hits = (self.hits or 0) + 1
    if self.health <= 0 then
      local arena = scene.find("/Arena01")
      if arena then arena.kills = (arena.kills or 0) + 1 end
      self:emit("died", { hits = self.hits })
      self:destroy()
      return
    end
  end

  local player = find_player(self)
  if not player then return end

  local to = player.pos - self.pos
  if to:length() > fx.new(12) then
    self:set_velocity(to:normalized() * fx.new(self.speed or 35))
  else
    self:set_velocity(vec2(fx.new(0), fx.new(0)))
  end
end
