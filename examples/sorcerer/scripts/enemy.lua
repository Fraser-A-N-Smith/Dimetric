-- An enemy that walks at the player and dies when its health runs out.
--
-- One script for every variant. What differs between a skeleton, a wraith and a
-- brute is numbers, and those come from the prefab's overrides rather than from
-- three near-identical files.

local function find_player(self)
  if self.player_id then
    local cached = scene.by_id(self.player_id)
    if cached then return cached end
  end
  local player = scene.find("/Arena01/Player")
  if player then self.player_id = player:id() end
  return player
end

-- The three variants, by the tag their prefab carries.
--
-- The stats are here rather than on the node because an instance override
-- reaches a node's *properties*, and a project cannot declare a property of its
-- own on a node kind — so authored per-instance numbers have nowhere to live.
-- Overrides still differentiate the variants visually (radius, tint); the
-- numbers come from a tag. See `docs/ENGINE-GAPS.md`.
local VARIANTS = {
  skeleton = { health = 40, speed = 35, touch = 4 },
  wraith = { health = 22, speed = 62, touch = 6 },
  brute = { health = 110, speed = 20, touch = 12 },
}

local function variant(self)
  for name, stats in pairs(VARIANTS) do
    if self:has_tag(name) then return stats end
  end
  return VARIANTS.skeleton
end

function on_ready(self)
  local stats = variant(self)
  self.health = stats.health
  self.speed = stats.speed
  self.touch_damage = stats.touch
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
