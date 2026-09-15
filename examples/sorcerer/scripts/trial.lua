-- A focused test of one spell, for an agent verifying its own work.
--
-- Spawns a single skeleton, holds the two spells whose combination is under
-- test, and fires at it. Short enough to read the result off a probe.

local spells = require("scripts/spellbook.lua")

local CENTRE = vec2(fx.new(160), fx.new(96))

function on_ready(self)
  self.spawned = false
  self.fired = 0
  self.cooldown = 0
end

function on_tick(self)
  if not self.spawned then
    self.spawned = true
    self.target_id = scene.spawn("prefabs/skeleton", CENTRE)
    return
  end

  local target = self.target_id and scene.by_id(self.target_id)
  if not target then return end

  -- The evolution under test, looked up the same way the arena does.
  local spell = spells.evolutions["bolt+frost"]
  if not spell then self.missing = true; return end
  self.spell_name = spell.name

  if (self.cooldown or 0) > 0 then
    self.cooldown = self.cooldown - 1
    return
  end

  -- Aim straight at it, so the test is about the spell and not about aiming.
  local player = scene.find("/Arena01/Player")
  if not player then return end
  local to = target.pos - player.pos
  local id = scene.spawn("prefabs/bolt", player.pos)
  local pending = self.pending or {}
  pending[#pending + 1] = { id = id, angle = to:angle_degrees(), spell = spell }
  self.pending = pending
  self.cooldown = spell.cooldown
  self.fired = (self.fired or 0) + 1
end

function on_post_tick(self)
  local pending = self.pending
  if not pending then return end
  local waiting = {}
  for i = 1, #pending do
    local node = scene.by_id(pending[i].id)
    if node then
      local spell = pending[i].spell
      node.life = spell.life
      node.damage = spell.damage
      node.speed = spell.speed
      node:set_velocity(fx.from_angle(pending[i].angle) * fx.new(spell.speed))
    else
      waiting[#waiting + 1] = pending[i]
    end
  end
  self.pending = waiting

  local target = self.target_id and scene.by_id(self.target_id)
  self.target_alive = target ~= nil
  self.target_health = target and target.health or 0
end
